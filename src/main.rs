use std::{
    collections::BTreeMap,
    env,
    error::Error,
    fs,
    path::PathBuf,
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::Duration,
};

use std::os::windows::process::CommandExt;

use winreg::{
    RegKey,
    enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY},
};

use windows_sys::Win32::{
    Foundation::{CloseHandle, ERROR_SUCCESS, HANDLE, WAIT_OBJECT_0},
    System::{
        Registry::{
            HKEY, HKEY_CURRENT_USER as WIN_HKCU, HKEY_LOCAL_MACHINE as WIN_HKLM, KEY_NOTIFY,
            KEY_WOW64_32KEY as WIN_KEY32, KEY_WOW64_64KEY as WIN_KEY64, REG_NOTIFY_CHANGE_LAST_SET,
            REG_NOTIFY_CHANGE_NAME, RegCloseKey, RegNotifyChangeKeyValue, RegOpenKeyExW,
        },
        Threading::{
            CreateEventW, EVENT_MODIFY_STATE, OpenEventW, SYNCHRONIZATION_SYNCHRONIZE, SetEvent,
            WaitForMultipleObjects,
        },
    },
};

type AppResult<T> = Result<T, Box<dyn Error>>;

const APP_NAME: &str = "SoftwareWatcher";

const UNINSTALL_PATH: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall";

const STOP_EVENT: &str = r"Local\SoftwareWatcher_Stop";

const CREATE_NO_WINDOW: u32 = 0x08000000;

// ============================================================
// Data structures
// ============================================================

#[derive(Clone)]
struct Software {
    name: String,
    version: String,
    publisher: String,
    location: String,
    install_date: String,

    // These fields are used internally and omitted from the full CSV.
    drive: String,
    category: String,

    reinstall: String,
    reason: String,
}

struct WingetPackage {
    name: String,
    id: String,
}

// ============================================================
// Classification
// ============================================================

fn contains_any(text: &str, words: &[&str]) -> bool {
    words.iter().any(|word| text.contains(word))
}

// Classifies software by its Registry metadata.
fn classify(
    name: &str,
    publisher: &str,
    location: &str,
    system_component: u32,
) -> (String, String, String) {
    let name = name.to_lowercase();
    let publisher = publisher.to_lowercase();
    let location = location.to_lowercase();

    if system_component == 1 {
        return (
            "SYSTEM_COMPONENT".into(),
            "NO".into(),
            "Marked by Windows as a system component".into(),
        );
    }

    if location.starts_with(r"c:\windows") {
        return (
            "SYSTEM_COMPONENT".into(),
            "NO".into(),
            "Located in the Windows system directory".into(),
        );
    }

    let runtimes = [
        "microsoft visual c++",
        ".net runtime",
        ".net host",
        ".net targeting",
        ".net sdk",
        "asp.net core",
        "desktop runtime",
        "windows sdk",
        "windows software development kit",
        "redistributable",
    ];

    if contains_any(&name, &runtimes) {
        return (
            "RUNTIME".into(),
            "NO".into(),
            "Runtime / SDK component".into(),
        );
    }

    let system = [
        "microsoft edge webview2",
        "windows app runtime",
        "microsoft update health",
        "windows update",
    ];

    if contains_any(&name, &system) {
        return (
            "SYSTEM_COMPONENT".into(),
            "NO".into(),
            "Windows / Microsoft system component".into(),
        );
    }

    // Do not classify GUI software such as NVIDIA App or AMD Software as a driver.
    let drivers = [
        "graphics driver",
        "display driver",
        "audio driver",
        "wireless driver",
        "bluetooth driver",
        "lan driver",
        "chipset driver",
        "amd chipset software",
    ];

    if contains_any(&name, &drivers) || name.ends_with(" driver") {
        return ("DRIVER".into(), "REVIEW".into(), "Hardware driver".into());
    }

    let support = [
        "application support",
        "bonjour",
        "shared framework",
        "support component",
        "update service",
        "updater",
    ];

    if contains_any(&name, &support) {
        return (
            "SUPPORT_COMPONENT".into(),
            "REVIEW".into(),
            "May be a component bundled with another program".into(),
        );
    }

    // Microsoft also publishes applications that users install explicitly.
    let microsoft_apps = [
        "visual studio code",
        "powertoys",
        "visual studio",
        "windows terminal",
        "sysinternals",
    ];

    if publisher.contains("microsoft") && contains_any(&name, &microsoft_apps) {
        return (
            "USER_APP".into(),
            "YES".into(),
            "Microsoft user application".into(),
        );
    }

    (
        "USER_APP".into(),
        "YES".into(),
        "General installed application".into(),
    )
}

// ============================================================
// Date and drive detection
// ============================================================

// Converts YYYYMMDD to YYYY-MM-DD.
fn format_install_date(date: &str) -> String {
    let date = date.trim();

    if date.len() == 8 && date.chars().all(|c| c.is_ascii_digit()) {
        format!("{}-{}-{}", &date[0..4], &date[4..6], &date[6..8])
    } else {
        date.to_string()
    }
}

// Extracts a drive letter such as C: or D: from a path or command line.
fn drive_from_text(text: &str) -> Option<String> {
    let bytes = text.as_bytes();

    for i in 0..bytes.len().saturating_sub(2) {
        if bytes[i].is_ascii_alphabetic()
            && bytes[i + 1] == b':'
            && (bytes[i + 2] == b'\\' || bytes[i + 2] == b'/')
        {
            return Some(format!("{}:", (bytes[i] as char).to_ascii_uppercase()));
        }
    }

    None
}

// Falls back to UninstallString or DisplayIcon when InstallLocation is empty.
fn detect_drive(location: &str, uninstall: &str, display_icon: &str) -> String {
    drive_from_text(location)
        .or_else(|| drive_from_text(uninstall))
        .or_else(|| drive_from_text(display_icon))
        .unwrap_or_default()
}

// ============================================================
// Registry scanning
// ============================================================

fn read_view(root: winreg::HKEY, flags: u32, apps: &mut BTreeMap<String, Software>) {
    let root = RegKey::predef(root);

    let Ok(uninstall) = root.open_subkey_with_flags(UNINSTALL_PATH, KEY_READ | flags) else {
        return;
    };

    for key_name in uninstall.enum_keys().flatten() {
        let Ok(key) = uninstall.open_subkey_with_flags(&key_name, KEY_READ) else {
            continue;
        };

        let Ok(name): Result<String, _> = key.get_value("DisplayName") else {
            continue;
        };

        if name.trim().is_empty() {
            continue;
        }

        let version: String = key.get_value("DisplayVersion").unwrap_or_default();

        let publisher: String = key.get_value("Publisher").unwrap_or_default();

        let location: String = key.get_value("InstallLocation").unwrap_or_default();

        let uninstall_string: String = key.get_value("UninstallString").unwrap_or_default();

        let display_icon: String = key.get_value("DisplayIcon").unwrap_or_default();

        let install_date: String = key.get_value("InstallDate").unwrap_or_default();

        let system_component: u32 = key.get_value("SystemComponent").unwrap_or(0);

        let install_date = format_install_date(&install_date);

        let drive = detect_drive(&location, &uninstall_string, &display_icon);

        let (category, reinstall, reason) =
            classify(&name, &publisher, &location, system_component);

        let app = Software {
            name: name.trim().into(),
            version: version.trim().into(),
            publisher: publisher.trim().into(),
            location: location.trim().into(),
            install_date,
            drive,
            category,
            reinstall,
            reason,
        };

        // Deduplicate entries across the 32-bit and 64-bit Registry views.
        let id = format!(
            "{}|{}|{}",
            app.name.to_lowercase(),
            app.version.to_lowercase(),
            app.publisher.to_lowercase()
        );

        apps.insert(id, app);
    }
}

fn scan_software() -> Vec<Software> {
    let mut apps = BTreeMap::new();

    read_view(HKEY_LOCAL_MACHINE, KEY_WOW64_64KEY, &mut apps);

    read_view(HKEY_LOCAL_MACHINE, KEY_WOW64_32KEY, &mut apps);

    read_view(HKEY_CURRENT_USER, KEY_WOW64_64KEY, &mut apps);

    read_view(HKEY_CURRENT_USER, KEY_WOW64_32KEY, &mut apps);

    let mut apps: Vec<_> = apps.into_values().collect();

    apps.sort_by_key(|app| app.name.to_lowercase());

    apps
}

// ============================================================
// Winget
// ============================================================

// Splits a winget table row into columns.
fn split_columns(line: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut current = String::new();
    let mut spaces = 0;

    for c in line.chars() {
        if c == ' ' {
            spaces += 1;
            continue;
        }

        if spaces >= 2 {
            if !current.trim().is_empty() {
                result.push(current.trim().to_string());
                current.clear();
            }
        } else if spaces == 1 && !current.is_empty() {
            current.push(' ');
        }

        spaces = 0;
        current.push(c);
    }

    if !current.trim().is_empty() {
        result.push(current.trim().to_string());
    }

    result
}

// Reads installed applications from the winget community source.
fn winget_packages() -> Option<Vec<WingetPackage>> {
    let output = Command::new("winget")
        .args([
            "list",
            "--source",
            "winget",
            "--include-unknown",
            "--accept-source-agreements",
            "--disable-interactivity",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let text = String::from_utf8_lossy(&output.stdout);

    let mut packages = Vec::new();
    let mut table_started = false;

    for line in text.lines() {
        let trimmed = line.trim();

        // Find the dashed separator below the winget table header.
        if !table_started {
            if trimmed.len() > 5
                && trimmed.contains("---")
                && trimmed.chars().all(|c| c == '-' || c == ' ')
            {
                table_started = true;
            }

            continue;
        }

        if trimmed.is_empty() {
            continue;
        }

        let columns = split_columns(line);

        if columns.len() >= 2 {
            packages.push(WingetPackage {
                name: columns[0].clone(),
                id: columns[1].clone(),
            });
        }
    }

    Some(packages)
}

// Removes spaces and symbols to make name matching more stable.
fn normalize_name(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

// Finds a winget package ID from a Registry DisplayName.
fn find_winget_id<'a>(app_name: &str, packages: &'a [WingetPackage]) -> Option<&'a str> {
    let target = normalize_name(app_name);

    packages
        .iter()
        .find(|package| normalize_name(&package.name) == target)
        .map(|package| package.id.as_str())
}

// ============================================================
// CSV
// ============================================================

fn csv_path(name: &str) -> AppResult<PathBuf> {
    let exe = env::current_exe()?;

    Ok(exe
        .parent()
        .ok_or("Unable to locate the executable directory")?
        .join(name))
}

fn write_if_changed(path: &PathBuf, data: Vec<u8>) -> AppResult<()> {
    if fs::read(path).ok().as_deref() != Some(&data) {
        fs::write(path, data)?;
    }

    Ok(())
}

// Full software inventory
fn write_software_csv(apps: &[Software]) -> AppResult<()> {
    let mut data = vec![0xEF, 0xBB, 0xBF];

    {
        let mut writer = csv::Writer::from_writer(&mut data);

        writer.write_record([
            "Name",
            "Category",
            "NeedReinstall",
            "Reason",
            "Version",
            "Publisher",
            "InstallDate",
            "InstallLocation",
        ])?;

        for app in apps {
            writer.write_record([
                &app.name,
                &app.category,
                &app.reinstall,
                &app.reason,
                &app.version,
                &app.publisher,
                &app.install_date,
                &app.location,
            ])?;
        }

        writer.flush()?;
    }

    write_if_changed(&csv_path("software.csv")?, data)
}

// Condensed reinstall list
fn write_reinstall_csv(
    apps: &[Software],
    winget: Option<&[WingetPackage]>,
) -> AppResult<(usize, usize)> {
    let system_drive = env::var("SystemDrive").unwrap_or_else(|_| "C:".into());

    let mut data = vec![0xEF, 0xBB, 0xBF];

    let mut total = 0;
    let mut found = 0;

    {
        let mut writer = csv::Writer::from_writer(&mut data);

        writer.write_record(["Name", "Version", "InstallDate", "InstallLocation"])?;

        for app in apps {
            // Include only:
            // 1. USER_APP
            // 2. Applications confirmed to be on the Windows system drive
            if app.category != "USER_APP" || !app.drive.eq_ignore_ascii_case(&system_drive) {
                continue;
            }

            total += 1;

            let (id, status) = match winget {
                Some(packages) => match find_winget_id(&app.name, packages) {
                    Some(id) => {
                        found += 1;
                        (id, "FOUND")
                    }

                    None => ("", "UNKNOWN"),
                },

                None => ("", "WINGET_UNAVAILABLE"),
            };

            writer.write_record([&app.name, &app.version, &app.install_date, &app.location])?;
        }

        writer.flush()?;
    }

    write_if_changed(&csv_path("reinstall.csv")?, data)?;

    Ok((total, found))
}

// Updates both CSV files after a scan or watcher event.
fn update_all(verbose: bool) -> AppResult<()> {
    let apps = scan_software();

    let winget = winget_packages();

    write_software_csv(&apps)?;

    let (reinstall_count, winget_found) = write_reinstall_csv(&apps, winget.as_deref())?;

    if verbose {
        let yes = apps.iter().filter(|a| a.reinstall == "YES").count();

        let review = apps.iter().filter(|a| a.reinstall == "REVIEW").count();

        let no = apps.iter().filter(|a| a.reinstall == "NO").count();

        println!("Found {} applications", apps.len());
        println!("Reinstall required: {yes}");
        println!("Review required: {review}");
        println!("Reinstall usually not required: {no}");

        println!();
        println!("Condensed reinstall list: {reinstall_count}");
        println!("Identified by winget: {winget_found}");
        println!(
            "Not identified by winget: {}",
            reinstall_count - winget_found
        );

        println!();
        println!("Full inventory: {}", csv_path("software.csv")?.display());

        println!("Condensed list: {}", csv_path("reinstall.csv")?.display());
    }

    Ok(())
}

// ============================================================
// Watcher
// ============================================================

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn open_stop_event() -> Option<HANDLE> {
    let name = wide(STOP_EVENT);

    let handle = unsafe {
        OpenEventW(
            EVENT_MODIFY_STATE | SYNCHRONIZATION_SYNCHRONIZE,
            0,
            name.as_ptr(),
        )
    };

    if handle.is_null() { None } else { Some(handle) }
}

fn running() -> bool {
    let Some(handle) = open_stop_event() else {
        return false;
    };

    unsafe {
        CloseHandle(handle);
    }

    true
}

// Each thread monitors one Registry view.
fn watch_registry(current_user: bool, wow_flag: u32, sender: mpsc::Sender<bool>) {
    let root: HKEY = if current_user { WIN_HKCU } else { WIN_HKLM };

    let path = wide(UNINSTALL_PATH);

    let mut key: HKEY = std::ptr::null_mut();

    if unsafe { RegOpenKeyExW(root, path.as_ptr(), 0, KEY_NOTIFY | wow_flag, &mut key) }
        != ERROR_SUCCESS
    {
        return;
    }

    let event = unsafe { CreateEventW(std::ptr::null(), 0, 0, std::ptr::null()) };

    let Some(stop) = open_stop_event() else {
        unsafe { RegCloseKey(key) };

        return;
    };

    if event.is_null() {
        unsafe {
            CloseHandle(stop);
            RegCloseKey(key);
        }

        return;
    }

    loop {
        let result = unsafe {
            RegNotifyChangeKeyValue(
                key,
                1,
                REG_NOTIFY_CHANGE_NAME | REG_NOTIFY_CHANGE_LAST_SET,
                event,
                1,
            )
        };

        if result != ERROR_SUCCESS {
            break;
        }

        let handles = [event, stop];

        let result = unsafe { WaitForMultipleObjects(2, handles.as_ptr(), 0, u32::MAX) };

        if result == WAIT_OBJECT_0 {
            let _ = sender.send(true);
        } else if result == WAIT_OBJECT_0 + 1 {
            let _ = sender.send(false);

            break;
        } else {
            break;
        }
    }

    unsafe {
        CloseHandle(event);
        CloseHandle(stop);
        RegCloseKey(key);
    }
}

// The background watcher rescans only after a change.
fn run_watcher() -> AppResult<()> {
    if running() {
        return Ok(());
    }

    let name = wide(STOP_EVENT);

    let stop_event = unsafe { CreateEventW(std::ptr::null(), 1, 0, name.as_ptr()) };

    if stop_event.is_null() {
        return Err("Unable to create the watcher event".into());
    }

    update_all(false)?;

    let (tx, rx) = mpsc::channel();

    for (user, flag) in [
        (false, WIN_KEY64),
        (false, WIN_KEY32),
        (true, WIN_KEY64),
        (true, WIN_KEY32),
    ] {
        let tx = tx.clone();

        thread::spawn(move || {
            watch_registry(user, flag, tx);
        });
    }

    drop(tx);

    while let Ok(changed) = rx.recv() {
        if !changed {
            break;
        }

        // Installers commonly update many Registry entries in succession.
        thread::sleep(Duration::from_secs(3));

        let mut stop_requested = false;

        // Drain duplicate Registry events accumulated during the three-second delay.
        while let Ok(next) = rx.try_recv() {
            if !next {
                stop_requested = true;
                break;
            }
        }

        if stop_requested {
            break;
        }

        update_all(false)?;
    }

    unsafe {
        CloseHandle(stop_event);
    }

    Ok(())
}

// Configures automatic startup when the user signs in to Windows.
// A hidden PowerShell process starts the watcher without showing a terminal window.
fn set_autostart(enable: bool) -> AppResult<()> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);

    let (run, _) = hkcu.create_subkey(r"Software\Microsoft\Windows\CurrentVersion\Run")?;

    if enable {
        let exe = env::current_exe()?;

        // Escape single quotes that may appear in the executable path.
        let exe_path = exe.to_string_lossy().replace('\'', "''");

        let command = format!(
            "powershell.exe -NoProfile -WindowStyle Hidden \
             -Command \"Start-Process -FilePath '{}' \
             -ArgumentList 'run' -WindowStyle Hidden\"",
            exe_path
        );

        run.set_value(APP_NAME, &command)?;
    } else {
        let _ = run.delete_value(APP_NAME);
    }

    Ok(())
}

fn start() -> AppResult<()> {
    if running() {
        println!("Software Watcher is already running.");

        return Ok(());
    }

    Command::new(env::current_exe()?)
        .arg("run")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()?;

    thread::sleep(Duration::from_millis(400));

    if running() {
        set_autostart(true)?;

        println!("Software Watcher started.");
    } else {
        println!("Failed to start the watcher.");
    }

    Ok(())
}

fn stop() -> AppResult<()> {
    let Some(handle) = open_stop_event() else {
        println!("Software Watcher is not running.");

        return Ok(());
    };

    unsafe {
        SetEvent(handle);
        CloseHandle(handle);
    }

    for _ in 0..40 {
        if !running() {
            println!("Software Watcher stopped.");

            return Ok(());
        }

        thread::sleep(Duration::from_millis(100));
    }

    println!("The stop request was sent.");

    Ok(())
}

// ============================================================
// main
// ============================================================

fn main() -> AppResult<()> {
    let command = env::args().nth(1);

    match command.as_deref() {
        Some("start") => {
            start()?;
        }

        Some("stop") => {
            stop()?;
        }

        Some("restart") => {
            stop()?;

            thread::sleep(Duration::from_millis(300));

            start()?;
        }

        Some("status") => {
            println!("Status: {}", if running() { "Running" } else { "Stopped" });
        }

        // A manual scan updates software.csv and reinstall.csv together.
        Some("scan") => {
            update_all(true)?;
        }

        Some("uninstall") => {
            stop()?;
            set_autostart(false)?;

            println!("Automatic startup was removed.");

            println!("software.csv and reinstall.csv will be preserved.");

            println!("You can now delete the executable.");
        }

        // Run the watcher in the background.
        Some("run") => {
            run_watcher()?;
        }

        _ => {
            println!("Software Watcher");

            println!("  start");
            println!("  stop");
            println!("  restart");
            println!("  status");
            println!("  scan");
            println!("  uninstall");
        }
    }

    Ok(())
}
