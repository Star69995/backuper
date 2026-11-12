//! Notification clicks through the `backuper://` address. Windows hands a click to an unpackaged
//! app's own handler only while the notification is still on screen, and not always then; once it
//! moved to the notification center a click does nothing. A notification with protocol activation
//! instead makes Windows open its address, i.e. start our exe with it: the running app gets it
//! from the single-instance callback (`lib.rs`), a fresh start reads it from its own args.

use crate::core::ToastTarget;
use std::path::Path;

pub const SCHEME: &str = "backuper";

/// Runs from a cargo build folder (`target\debug` / `target\release`): Windows doesn't know our
/// app id there, so notifications go out under PowerShell's and the address isn't registered.
pub fn from_build_folder() -> bool {
    std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)).is_some_and(|dir| {
        dir.parent().is_some_and(|t| t.ends_with("target")) && (dir.ends_with("debug") || dir.ends_with("release"))
    })
}

/// The address a click on a notification about `target` opens.
pub fn uri(target: &ToastTarget) -> String {
    let path = match target {
        ToastTarget::Window => "window".to_string(),
        ToastTarget::Tasks => "tasks".to_string(),
        ToastTarget::Settings => "settings".to_string(),
        ToastTarget::Update => "update".to_string(),
        ToastTarget::Run(id) => format!("run/{id}"),
    };
    format!("{SCHEME}://{path}")
}

/// The target of an address from `uri` (Windows may add a trailing slash).
pub fn parse(arg: &str) -> Option<ToastTarget> {
    let rest = arg.strip_prefix(SCHEME)?.strip_prefix("://")?.trim_end_matches('/');
    Some(match rest {
        "window" => ToastTarget::Window,
        "tasks" => ToastTarget::Tasks,
        "settings" => ToastTarget::Settings,
        "update" => ToastTarget::Update,
        _ => {
            let id = rest.strip_prefix("run/")?;
            if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
                return None;
            }
            ToastTarget::Run(id.to_string())
        }
    })
}

/// The notification target among a process's args, if it was started by a click.
pub fn from_args<S: AsRef<str>>(args: &[S]) -> Option<ToastTarget> {
    args.iter().find_map(|a| parse(a.as_ref()))
}

/// Points `backuper://` at this exe (per user), as the installer does - kept in line on every
/// start, so a click opens this copy. Not from a build folder.
#[cfg(windows)]
pub fn register() {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;
    if from_build_folder() {
        return;
    }
    let Ok(exe) = std::env::current_exe() else { return };
    let command = format!("\"{}\" \"%1\"", exe.display());
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let base = format!(r"Software\Classes\{SCHEME}");
    if let Ok(key) = hkcu.open_subkey(format!(r"{base}\shell\open\command")) {
        if key.get_value::<String, _>("").is_ok_and(|v| v == command) {
            return;
        }
    }
    let write = || -> std::io::Result<()> {
        let (key, _) = hkcu.create_subkey(&base)?;
        key.set_value("", &"URL:Backuper")?;
        key.set_value("URL Protocol", &"")?;
        let (icon, _) = hkcu.create_subkey(format!(r"{base}\DefaultIcon"))?;
        icon.set_value("", &format!("\"{}\",0", exe.display()))?;
        let (cmd, _) = hkcu.create_subkey(format!(r"{base}\shell\open\command"))?;
        cmd.set_value("", &command)
    };
    let _ = write();
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

/// The notification's XML: a click opens `launch`. `sound` is a toast sound name (as in
/// tauri-winrt-notification's `Sound`); unknown or empty = silent.
pub fn toast_xml(title: &str, body: &str, sound: &str, launch: &str) -> String {
    use tauri_winrt_notification::Sound;
    let audio = match sound.parse::<Sound>() {
        Ok(Sound::Single(s) | Sound::Loop(s)) => format!(r#"<audio src="ms-winsoundevent:Notification.Looping.{s}"/>"#),
        Ok(s) => format!(r#"<audio src="ms-winsoundevent:Notification.{s}"/>"#),
        Err(_) => r#"<audio silent="true"/>"#.to_string(),
    };
    format!(
        r#"<toast activationType="protocol" launch="{}"><visual><binding template="ToastGeneric"><text>{}</text><text>{}</text></binding></visual>{audio}</toast>"#,
        escape(launch),
        escape(title),
        escape(body)
    )
}

/// Shows a notification under `app_id` whose click opens the address of `target`.
#[cfg(windows)]
pub fn show(app_id: &str, title: &str, body: &str, sound: &str, target: &ToastTarget) -> windows::core::Result<()> {
    use windows::core::HSTRING;
    use windows::Data::Xml::Dom::XmlDocument;
    use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};
    let doc = XmlDocument::new()?;
    doc.LoadXml(&HSTRING::from(toast_xml(title, body, sound, &uri(target))))?;
    let toast = ToastNotification::CreateToastNotification(&doc)?;
    ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(app_id))?.Show(&toast)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uri_round_trip() {
        let id = "5f0c2a1e-3b4d-4c5e-8f60-718293a4b5c6".to_string();
        for t in [ToastTarget::Window, ToastTarget::Tasks, ToastTarget::Settings, ToastTarget::Update, ToastTarget::Run(id)] {
            assert_eq!(parse(&uri(&t)), Some(t.clone()));
            assert_eq!(parse(&format!("{}/", uri(&t))), Some(t));
        }
        assert_eq!(parse("backuper://run/a b"), None);
        assert_eq!(parse("backuper://run/"), None);
        assert_eq!(parse("backuper://other"), None);
        assert_eq!(parse("--hidden"), None);
        assert_eq!(from_args(&["C:\\x\\backuper.exe", "backuper://tasks"]), Some(ToastTarget::Tasks));
    }

    #[test]
    fn xml_escapes_and_picks_the_sound() {
        let xml = toast_xml("a < b & \"c\"", "x", "Reminder", "backuper://tasks");
        assert!(xml.contains("<text>a &lt; b &amp; &quot;c&quot;</text>"));
        assert!(xml.contains(r#"activationType="protocol" launch="backuper://tasks""#));
        assert!(xml.contains("Notification.Reminder"));
        assert!(toast_xml("t", "b", "Alarm2", "x").contains("Notification.Looping.Alarm2"));
        assert!(toast_xml("t", "b", "", "x").contains(r#"<audio silent="true"/>"#));
    }
}
