\# Software Watcher



A lightweight Windows utility written in Rust that monitors installed software and automatically maintains software inventory and reinstall lists.



Software Watcher is especially useful before reinstalling Windows. Instead of trying to remember every application you installed, it keeps an up-to-date record automatically.



\## Features



\- Automatically detects software installation and removal

\- Event-driven monitoring with no continuous polling

\- Runs quietly in the background

\- Starts automatically with Windows

\- Generates a complete software inventory

\- Generates a simplified reinstall list

\- Supports manual scanning

\- Portable single executable

\- No GUI

\- No tray icon

\- Windows only



\## Download



Download `software-watcher.exe` from the GitHub Releases page.



No installer is required.



Place the executable in a permanent folder that will not be removed when Windows is reinstalled.



For example:



```text

E:\\software-watcher\\

```



or:



```text

D:\\software-watcher\\

```



The CSV files will be created in the same folder as the executable.



\## Quick Start



Open PowerShell in the folder containing `software-watcher.exe`.



Start the watcher:



```powershell

.\\software-watcher.exe start

```



Check its status:



```powershell

.\\software-watcher.exe status

```



Expected output:



```text

Status: Running

```



Once started, Software Watcher runs in the background and automatically starts again when you sign in to Windows.



\## Commands



\### Start



```powershell

.\\software-watcher.exe start

```



Starts the background watcher and creates the Windows startup entry.



\### Stop



```powershell

.\\software-watcher.exe stop

```



Stops the background watcher.



The Windows startup entry is kept, so the watcher can start again the next time you sign in.



\### Restart



```powershell

.\\software-watcher.exe restart

```



Stops and starts the watcher again.



\### Status



```powershell

.\\software-watcher.exe status

```



Shows whether the watcher is currently running.



Example:



```text

Status: Running

```



or:



```text

Status: Stopped

```



\### Scan



```powershell

.\\software-watcher.exe scan

```



Immediately scans the currently installed software and updates the CSV files.



This does not start another background watcher.



\### Uninstall



```powershell

.\\software-watcher.exe uninstall

```



Stops the watcher and removes its Windows startup entry.



The following files are not automatically deleted:



```text

software-watcher.exe

software.csv

reinstall.csv

```



You can delete them manually afterward if you no longer need them.



\## Output Files



Software Watcher creates two CSV files next to the executable.



\### `software.csv`



A complete software inventory.



It may contain:



\- User applications

\- Drivers

\- Runtimes

\- Support components

\- Windows or system components



Typical information includes:



```text

Name

Category

NeedReinstall

Reason

Version

Publisher

InstallDate

InstallLocation

```



This file is intended to act as the complete software record of the machine.



\### `reinstall.csv`



A simplified list intended for Windows reinstallation.



It focuses on normal user applications installed on the Windows system drive and excludes most items such as:



\- Drivers

\- Runtimes

\- System components

\- Support components

\- Applications installed on non-system drives



The goal is to provide a cleaner list of applications that may need to be installed again after reinstalling Windows.



\## Why Use It?



After using a Windows installation for a long time, it is easy to forget small utilities and less frequently used applications.



Software Watcher keeps a record automatically so that before reinstalling Windows you can simply keep:



```text

software.csv

reinstall.csv

```



on another drive or in cloud storage.



After reinstalling Windows, open `reinstall.csv` and use it as a checklist for restoring your applications.



\## How It Works



Software Watcher monitors the Windows uninstall registry.



It watches both 32-bit and 64-bit uninstall registry views under the current user and local machine.



The watcher uses Windows registry change notifications rather than repeatedly scanning the system.



In simplified form:



```text

Software installed or removed

&#x20;       ↓

Windows uninstall registry changes

&#x20;       ↓

Software Watcher receives an event

&#x20;       ↓

Installed software is rescanned

&#x20;       ↓

CSV files are updated

```



Because the watcher is event-driven, it normally spends its idle time waiting for Windows events instead of continuously polling the registry.



\## Windows Startup



Running:



```powershell

.\\software-watcher.exe start

```



also adds Software Watcher to the current user's Windows startup entries.



The watcher is launched automatically after signing in to Windows.



Running:



```powershell

.\\software-watcher.exe stop

```



only stops the current background process.



To stop the watcher and remove automatic startup, use:



```powershell

.\\software-watcher.exe uninstall

```



\## Limitations



Software Watcher primarily relies on software information registered in the Windows uninstall registry.



Because of this:



\- Portable applications may not appear

\- Some applications may not provide an install date

\- Some applications may not provide an install location

\- Some installers may register multiple components

\- Software classification is heuristic and may not always be perfect

\- Driver and system-component detection may not cover every possible package

\- Applications installed outside the system drive are intentionally excluded from the simplified reinstall list



The complete `software.csv` should be used when you need the full inventory.



\## Privacy



Software Watcher works locally.



The generated CSV files can contain information such as:



\- Installed application names

\- Versions

\- Publishers

\- Installation paths

\- Windows user folder paths



Review these files before sharing them publicly.



It is recommended that `software.csv` and `reinstall.csv` are not committed to a public Git repository.



\## SmartScreen



Prebuilt releases may be unsigned.



Windows SmartScreen may therefore show an "Unknown publisher" warning when running the executable for the first time.



The source code is available in this repository for inspection.



\## Building from Source



This section is only required if you want to compile Software Watcher yourself.



Requirements:



\- Windows

\- Rust

\- Cargo



Clone the repository and build the release version:



```powershell

cargo build --release

```



The executable will be created at:



```text

target\\release\\software-watcher.exe

```



Check the project without producing a release build:



```powershell

cargo check

```



Format the Rust source:



```powershell

cargo fmt

```



\## Recommended `.gitignore`



The repository should not include build output or personal CSV inventories.



Example:



```gitignore

/target

software.csv

reinstall.csv

\*.bak

```



\## Platform



```text

Windows only

```



\## License



See the `LICENSE` file for license information.

