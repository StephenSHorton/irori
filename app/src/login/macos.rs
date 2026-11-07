//! macOS launch at login (Wails autostart_darwin.go,
//! autostart_darwin_smappservice.go): `SMAppService.mainAppService` from a
//! bundled app on macOS 13+, else the LaunchAgent plist. Not yet built or
//! run on the Mac (the PC has no macOS target); check it there.

use std::path::PathBuf;

use objc2::runtime::AnyClass;
use objc2_foundation::NSBundle;
use objc2_service_management::{SMAppService, SMAppServiceStatus};

use super::launchagent::Agents;
use super::resolved_exe;

pub struct Mac {
    agents: Agents,
}

enum Sm {
    Unavailable,
    NotRegistered,
    Enabled,
    RequiresApproval,
}

impl Mac {
    pub fn system() -> Mac {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default();
        Mac {
            agents: Agents {
                dir: home.join("Library").join("LaunchAgents"),
            },
        }
    }

    /// Wails `strategy`: SMAppService only from a bundle with an id on 13+
    /// (the class exists from 13 on).
    fn use_sm(&self) -> bool {
        running_from_bundle() && !bundle_id().is_empty() && AnyClass::get(c"SMAppService").is_some()
    }

    pub fn is_enabled(&self) -> Result<bool, String> {
        if self.use_sm() {
            // Requires approval means the person turned it off in System
            // Settings: not enabled, and the LaunchAgent still gets a look.
            if let Sm::Enabled = sm_status() {
                return Ok(true);
            }
        }
        Ok(self.agents.find(&resolved_exe()?)?.is_some())
    }

    pub fn enable(&self, label: &str, args: &[&str]) -> Result<(), String> {
        super::winrun::validate_identifier(label)?;
        if self.use_sm() {
            // SAFETY: plain ServiceManagement calls on macOS 13+.
            let r = unsafe { SMAppService::mainAppService().registerAndReturnError() };
            return r.map_err(|e| format!("SMAppService register: {}", e.localizedDescription()));
        }
        let path = self.agents.enable(label, &resolved_exe()?, args)?;
        // Best effort: active now for this session (Wails launchctlBootstrap).
        let _ = launchctl("bootstrap", &path);
        Ok(())
    }

    pub fn disable(&self) -> Result<(), String> {
        let mut errs = Vec::new();
        if self.use_sm() && matches!(sm_status(), Sm::Enabled | Sm::RequiresApproval) {
            // SAFETY: as in enable.
            let r = unsafe { SMAppService::mainAppService().unregisterAndReturnError() };
            if let Err(e) = r {
                errs.push(format!(
                    "SMAppService unregister: {}",
                    e.localizedDescription()
                ));
            }
        }
        if let Err(e) = self.agents.disable(&resolved_exe()?, |p| {
            let _ = launchctl("bootout", p);
        }) {
            errs.push(e);
        }
        if errs.is_empty() {
            Ok(())
        } else {
            Err(errs.join("\n"))
        }
    }
}

fn sm_status() -> Sm {
    if AnyClass::get(c"SMAppService").is_none() {
        return Sm::Unavailable;
    }
    // SAFETY: the class exists (macOS 13+).
    let st = unsafe { SMAppService::mainAppService().status() };
    if st == SMAppServiceStatus::Enabled {
        Sm::Enabled
    } else if st == SMAppServiceStatus::RequiresApproval {
        Sm::RequiresApproval
    } else {
        Sm::NotRegistered
    }
}

fn bundle_id() -> String {
    NSBundle::mainBundle()
        .bundleIdentifier()
        .map(|s| s.to_string())
        .unwrap_or_default()
}

/// Wails `runningFromAppBundle`: the exe is `<X>.app/Contents/MacOS/<name>`.
fn running_from_bundle() -> bool {
    let Ok(exe) = resolved_exe() else {
        return false;
    };
    let exe = std::path::Path::new(&exe);
    let Some(macos) = exe.parent() else {
        return false;
    };
    let Some(contents) = macos.parent() else {
        return false;
    };
    let Some(app) = contents.parent() else {
        return false;
    };
    macos.file_name().is_some_and(|n| n == "MacOS")
        && contents.file_name().is_some_and(|n| n == "Contents")
        && app.to_string_lossy().ends_with(".app")
}

fn launchctl(verb: &str, plist: &std::path::Path) -> std::io::Result<std::process::ExitStatus> {
    // SAFETY: getuid cannot fail.
    let uid = unsafe { libc::getuid() };
    std::process::Command::new("launchctl")
        .arg(verb)
        .arg(format!("gui/{uid}"))
        .arg(plist)
        .status()
}
