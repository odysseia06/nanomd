//! `nanomd --register` / `--unregister`: make nano.md the current user's
//! app for `.md` and `.markdown` files, without admin rights.

pub use imp::{register, unregister};

#[cfg(windows)]
mod imp {
    use std::io;
    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, WIN32_ERROR};
    use windows_sys::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey,
        RegCreateKeyExW, RegDeleteKeyValueW, RegDeleteTreeW, RegSetValueExW,
    };
    use windows_sys::Win32::UI::Shell::{SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify};

    const ROOT: &str = r"Software";
    const PROG_ID: &str = "nanomd.md";
    /// The name Settings and Open with show, and the key of nano.md's
    /// Default apps page.
    const APP_NAME: &str = "nano.md";
    const EXTENSIONS: [&str; 2] = [".md", ".markdown"];

    /// One registry value under `HKCU`; an empty name is the key's default.
    pub(super) struct Value {
        pub key: String,
        pub name: &'static str,
        pub data: String,
    }

    /// The keys nano.md owns, deleted whole when unregistering.
    fn own_keys(root: &str) -> [String; 2] {
        [
            format!(r"{root}\Classes\{PROG_ID}"),
            format!(r"{root}\nanomd"),
        ]
    }

    /// nano.md's values in keys other apps share, as (key, name).
    fn shared_values(root: &str) -> Vec<(String, &'static str)> {
        let mut out = vec![(format!(r"{root}\RegisteredApplications"), APP_NAME)];
        for ext in EXTENSIONS {
            out.push((format!(r"{root}\Classes\{ext}\OpenWithProgids"), PROG_ID));
        }
        out
    }

    /// Everything registering writes under `HKCU\<root>`: a ProgID that
    /// opens files with `exe`, its name for Open with, both extensions
    /// listing it, and the Capabilities entry that puts nano.md in
    /// Settings > Default apps.
    pub(super) fn values(root: &str, exe: &str) -> Vec<Value> {
        let [prog, app] = own_keys(root);
        let caps = format!(r"{app}\Capabilities");
        let value = |key: &str, name, data: &str| Value {
            key: key.to_owned(),
            name,
            data: data.to_owned(),
        };
        let mut out = vec![
            value(&prog, "", "Markdown document"),
            value(&format!(r"{prog}\Application"), "ApplicationName", APP_NAME),
            value(&format!(r"{prog}\DefaultIcon"), "", &format!("\"{exe}\",0")),
            value(
                &format!(r"{prog}\shell\open\command"),
                "",
                &format!("\"{exe}\" \"%1\""),
            ),
            value(&caps, "ApplicationName", APP_NAME),
            value(
                &caps,
                "ApplicationDescription",
                "Markdown viewer and editor",
            ),
        ];
        for ext in EXTENSIONS {
            out.push(value(&format!(r"{caps}\FileAssociations"), ext, PROG_ID));
        }
        for (key, name) in shared_values(root) {
            let data = if name == APP_NAME { caps.as_str() } else { "" };
            out.push(value(&key, name, data));
        }
        out
    }

    pub(super) fn set_all(root: &str, exe: &str) -> io::Result<()> {
        values(root, exe).iter().try_for_each(set)
    }

    pub(super) fn remove_all(root: &str) -> io::Result<()> {
        for key in own_keys(root) {
            delete_tree(&key)?;
        }
        for (key, name) in shared_values(root) {
            let (k, n) = (wide(&key), wide(name));
            // SAFETY: both are NUL-terminated and outlive the call.
            let rc = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, k.as_ptr(), n.as_ptr()) };
            ok_if_missing(rc).map_err(|e| context(&key, e))?;
        }
        Ok(())
    }

    pub fn register() -> io::Result<String> {
        let exe = std::env::current_exe()?;
        set_all(ROOT, &exe.display().to_string())?;
        refresh_explorer();
        // Windows 10 and 11 leave the default to the user: open the page
        // where they pick it.
        let page = format!("ms-settings:defaultapps?registeredAppUser={APP_NAME}");
        let _ = std::process::Command::new("explorer.exe").arg(page).spawn();
        Ok("nano.md is registered for .md and .markdown files. \
            In the Settings page that opened, choose it as the default for both."
            .to_owned())
    }

    pub fn unregister() -> io::Result<String> {
        remove_all(ROOT)?;
        refresh_explorer();
        Ok("nano.md is no longer registered for .md and .markdown files.".to_owned())
    }

    fn refresh_explorer() {
        // SAFETY: no pointers are passed.
        unsafe {
            SHChangeNotify(
                SHCNE_ASSOCCHANGED as i32,
                SHCNF_IDLIST,
                std::ptr::null(),
                std::ptr::null(),
            )
        };
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    fn context(key: &str, e: io::Error) -> io::Error {
        io::Error::new(e.kind(), format!(r"HKEY_CURRENT_USER\{key}: {e}"))
    }

    fn check(rc: WIN32_ERROR) -> io::Result<()> {
        if rc == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(io::Error::from_raw_os_error(rc as i32))
        }
    }

    fn ok_if_missing(rc: WIN32_ERROR) -> io::Result<()> {
        if rc == ERROR_FILE_NOT_FOUND {
            Ok(())
        } else {
            check(rc)
        }
    }

    pub(super) fn delete_tree(key: &str) -> io::Result<()> {
        let k = wide(key);
        // SAFETY: NUL-terminated and outlives the call.
        let rc = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, k.as_ptr()) };
        ok_if_missing(rc).map_err(|e| context(key, e))
    }

    fn set(v: &Value) -> io::Result<()> {
        let (key, name, data) = (wide(&v.key), wide(v.name), wide(&v.data));
        let mut h: HKEY = std::ptr::null_mut();
        // SAFETY: the wide strings are NUL-terminated and outlive the calls;
        // `data` is passed with its length in bytes, terminator included;
        // the key handle is closed before returning.
        let rc = unsafe {
            let rc = RegCreateKeyExW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                0,
                std::ptr::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_SET_VALUE,
                std::ptr::null(),
                &mut h,
                std::ptr::null_mut(),
            );
            if rc != ERROR_SUCCESS {
                rc
            } else {
                let rc = RegSetValueExW(
                    h,
                    name.as_ptr(),
                    0,
                    REG_SZ,
                    data.as_ptr().cast(),
                    (data.len() * 2) as u32,
                );
                RegCloseKey(h);
                rc
            }
        };
        check(rc).map_err(|e| context(&v.key, e))
    }

    #[cfg(test)]
    pub(super) fn get(key: &str, name: &str) -> Option<String> {
        use windows_sys::Win32::System::Registry::{RRF_RT_REG_SZ, RegGetValueW};
        let (k, n) = (wide(key), wide(name));
        let mut buf = [0u16; 512];
        let mut len = (buf.len() * 2) as u32;
        // SAFETY: `buf` holds `len` bytes; the strings are NUL-terminated.
        let rc = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                k.as_ptr(),
                n.as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                buf.as_mut_ptr().cast(),
                &mut len,
            )
        };
        (rc == ERROR_SUCCESS).then(|| String::from_utf16_lossy(&buf[..len as usize / 2 - 1]))
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use std::ffi::c_void;
    use std::io;
    use std::os::unix::ffi::OsStrExt;
    use std::path::{Path, PathBuf};

    type CfRef = *const c_void;

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFStringCreateWithBytes(
            alloc: CfRef,
            bytes: *const u8,
            len: isize,
            encoding: u32,
            external: u8,
        ) -> CfRef;
        fn CFURLCreateFromFileSystemRepresentation(
            alloc: CfRef,
            path: *const u8,
            len: isize,
            is_directory: u8,
        ) -> CfRef;
        fn CFEqual(a: CfRef, b: CfRef) -> u8;
        fn CFRelease(cf: CfRef);
    }

    #[link(name = "CoreServices", kind = "framework")]
    unsafe extern "C" {
        fn LSRegisterURL(url: CfRef, update: u8) -> i32;
        fn LSSetDefaultRoleHandlerForContentType(
            content_type: CfRef,
            role: u32,
            handler: CfRef,
        ) -> i32;
        fn LSCopyDefaultRoleHandlerForContentType(content_type: CfRef, role: u32) -> CfRef;
    }

    const UTF8: u32 = 0x0800_0100;
    const ALL_ROLES: u32 = 0xFFFF_FFFF;
    /// `CFBundleIdentifier` in assets/macos/Info.plist.
    const BUNDLE_ID: &str = "io.github.odysseia06.nanomd";
    /// The type Info.plist declares for `.md` and `.markdown`.
    const MARKDOWN: &str = "net.daringfireball.markdown";
    const TEXTEDIT: &str = "com.apple.TextEdit";

    /// An owned Core Foundation object.
    struct Cf(CfRef);

    impl Cf {
        fn new(r: CfRef) -> io::Result<Self> {
            if r.is_null() {
                Err(io::Error::other(
                    "Core Foundation could not create an object",
                ))
            } else {
                Ok(Self(r))
            }
        }

        fn string(s: &str) -> io::Result<Self> {
            // SAFETY: `s` is valid UTF-8 of the given length.
            Self::new(unsafe {
                CFStringCreateWithBytes(std::ptr::null(), s.as_ptr(), s.len() as isize, UTF8, 0)
            })
        }
    }

    impl Drop for Cf {
        fn drop(&mut self) {
            // SAFETY: created by a CF Create or Copy function, released once.
            unsafe { CFRelease(self.0) }
        }
    }

    fn status(code: i32) -> io::Result<()> {
        if code == 0 {
            Ok(())
        } else {
            Err(io::Error::other(format!("Launch Services error {code}")))
        }
    }

    fn set_handler(bundle_id: &str) -> io::Result<()> {
        let (uti, id) = (Cf::string(MARKDOWN)?, Cf::string(bundle_id)?);
        // SAFETY: both are live CFStrings.
        status(unsafe { LSSetDefaultRoleHandlerForContentType(uti.0, ALL_ROLES, id.0) })
    }

    fn handler_is(bundle_id: &str) -> io::Result<bool> {
        let (uti, id) = (Cf::string(MARKDOWN)?, Cf::string(bundle_id)?);
        // SAFETY: `uti` is a live CFString; the result is ours to release.
        let current = unsafe { LSCopyDefaultRoleHandlerForContentType(uti.0, ALL_ROLES) };
        let Ok(current) = Cf::new(current) else {
            return Ok(false); // no handler at all
        };
        // SAFETY: both are live CFStrings.
        Ok(unsafe { CFEqual(current.0, id.0) } != 0)
    }

    /// The `.app` this executable runs from: `nanomd.app/Contents/MacOS/nanomd`.
    /// Resolves symlinks, such as a Homebrew one on the PATH.
    fn app_bundle() -> io::Result<PathBuf> {
        let exe = std::env::current_exe()?.canonicalize()?;
        exe.ancestors()
            .nth(3)
            .filter(|p| p.extension().is_some_and(|e| e == "app"))
            .map(Path::to_owned)
            .ok_or_else(|| {
                io::Error::other("--register needs the app: run the nanomd inside nanomd.app")
            })
    }

    pub fn register() -> io::Result<String> {
        let app = app_bundle()?;
        let path = app.as_os_str().as_bytes();
        // SAFETY: `path` is valid for its length.
        let url = Cf::new(unsafe {
            CFURLCreateFromFileSystemRepresentation(
                std::ptr::null(),
                path.as_ptr(),
                path.len() as isize,
                1,
            )
        })?;
        // SAFETY: `url` is a live CFURL.
        status(unsafe { LSRegisterURL(url.0, 1) })?;
        set_handler(BUNDLE_ID)?;
        Ok("nano.md now opens .md and .markdown files.".to_owned())
    }

    pub fn unregister() -> io::Result<String> {
        // Only undo our own choice, not one the user made since.
        if !handler_is(BUNDLE_ID)? {
            return Ok("nano.md isn't the app for .md files; nothing to undo.".to_owned());
        }
        set_handler(TEXTEDIT)?;
        Ok("TextEdit opens .md and .markdown files again.".to_owned())
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod imp {
    use std::io;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    const DESKTOP: &str = "nanomd.desktop";
    const TYPES: [&str; 2] = ["text/markdown", "text/x-markdown"];

    /// An XDG base directory: `var` if it holds an absolute path (the spec
    /// ignores relative ones), else `fallback` under HOME.
    fn xdg_dir(var: &str, fallback: &str) -> io::Result<PathBuf> {
        let path = |v: &str| std::env::var_os(v).map(PathBuf::from);
        path(var)
            .filter(|p| p.is_absolute())
            .or_else(|| {
                path("HOME")
                    .filter(|h| h.is_absolute())
                    .map(|h| h.join(fallback))
            })
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is not set"))
    }

    fn data_home() -> io::Result<PathBuf> {
        xdg_dir("XDG_DATA_HOME", ".local/share")
    }

    fn icon_path(data: &Path) -> PathBuf {
        data.join("icons/hicolor/48x48/apps/nanomd.png")
    }

    /// `exe` as one argument of an Exec key: quoted with the spec's
    /// reserved characters escaped, `%` doubled, then escaped again as a
    /// string value.
    pub(super) fn exec_arg(exe: &Path) -> String {
        let mut arg = String::from("\"");
        for c in exe.to_string_lossy().chars() {
            match c {
                '"' | '`' | '$' | '\\' => arg.push('\\'),
                '%' => arg.push('%'),
                _ => {}
            }
            arg.push(c);
        }
        arg.push('"');
        arg.replace('\\', "\\\\")
    }

    pub(super) fn desktop_entry(exe: &Path) -> String {
        format!(
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=nano.md\n\
             Comment=Markdown viewer and editor\n\
             Exec={} %f\n\
             Icon=nanomd\n\
             Terminal=false\n\
             Categories=Utility;TextEditor;\n\
             MimeType={};\n",
            exec_arg(exe),
            TYPES.join(";")
        )
    }

    /// `mimeapps.list` without `desktop` in any association; a line left
    /// with no apps goes too. Other lines are kept as they were.
    pub(super) fn without(list: &str, desktop: &str) -> String {
        let mut out = String::new();
        for line in list.lines() {
            if let Some((key, apps)) = line.split_once('=')
                && !line.starts_with('#')
            {
                let all: Vec<&str> = apps.split(';').filter(|a| !a.is_empty()).collect();
                let kept: Vec<&str> = all.iter().copied().filter(|a| *a != desktop).collect();
                if kept.is_empty() && !all.is_empty() {
                    continue;
                }
                if kept.len() != all.len() {
                    out.push_str(&format!("{key}={};\n", kept.join(";")));
                    continue;
                }
            }
            out.push_str(line);
            out.push('\n');
        }
        out
    }

    fn run(program: &str, args: &[&str]) -> io::Result<()> {
        let status = Command::new(program)
            .args(args)
            .status()
            .map_err(|e| io::Error::new(e.kind(), format!("could not run {program}: {e}")))?;
        if status.success() {
            Ok(())
        } else {
            Err(io::Error::other(format!("{program} failed ({status})")))
        }
    }

    pub fn register() -> io::Result<String> {
        let exe = std::env::current_exe()?;
        let data = data_home()?;
        let apps = data.join("applications");
        std::fs::create_dir_all(&apps)?;
        std::fs::write(apps.join(DESKTOP), desktop_entry(&exe))?;
        let icon = icon_path(&data);
        if let Some(dir) = icon.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&icon, crate::APP_ICON)?;
        let _ = run("update-desktop-database", &[&apps.to_string_lossy()]);
        let mut args = vec!["default", DESKTOP];
        args.extend(TYPES);
        run("xdg-mime", &args)?;
        Ok("nano.md now opens .md and .markdown files.".to_owned())
    }

    pub fn unregister() -> io::Result<String> {
        let data = data_home()?;
        let apps = data.join("applications");
        for file in [apps.join(DESKTOP), icon_path(&data)] {
            match std::fs::remove_file(file) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
                _ => {}
            }
        }
        let list = xdg_dir("XDG_CONFIG_HOME", ".config")?.join("mimeapps.list");
        if let Ok(old) = std::fs::read_to_string(&list) {
            std::fs::write(&list, without(&old, DESKTOP))?;
        }
        let _ = run("update-desktop-database", &[&apps.to_string_lossy()]);
        Ok("nano.md is no longer the app for .md and .markdown files.".to_owned())
    }
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    #[test]
    fn registering_writes_every_value_and_unregistering_removes_them() {
        use super::imp::{delete_tree, get, remove_all, set_all, values};
        // A throwaway root, so the test never touches real associations.
        let root = format!(r"Software\nanomd-test-{}", std::process::id());
        struct Cleanup<'a>(&'a str);
        impl Drop for Cleanup<'_> {
            fn drop(&mut self) {
                let _ = delete_tree(self.0); // whatever the assertions do
            }
        }
        let _cleanup = Cleanup(&root);
        let exe = r"C:\Program Files\nano md\nanomd.exe";

        set_all(&root, exe).unwrap();
        let written = values(&root, exe);
        for v in &written {
            assert_eq!(
                get(&v.key, v.name).as_deref(),
                Some(v.data.as_str()),
                r"{}\{}",
                v.key,
                v.name
            );
        }
        let command = written
            .iter()
            .find(|v| v.key.ends_with(r"shell\open\command"));
        assert_eq!(command.unwrap().data, format!("\"{exe}\" \"%1\""));

        remove_all(&root).unwrap();
        let left: Vec<_> = written.iter().filter_map(|v| get(&v.key, v.name)).collect();
        assert!(left.is_empty(), "left behind: {left:?}");
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn exec_lines_quote_paths_with_spaces_and_reserved_characters() {
        use super::imp::{desktop_entry, exec_arg};
        use std::path::Path;
        assert_eq!(
            exec_arg(Path::new("/opt/nano md/nanomd")),
            r#""/opt/nano md/nanomd""#
        );
        assert_eq!(exec_arg(Path::new("/x/$a\"b")), r#""/x/\\$a\\"b""#);
        assert_eq!(exec_arg(Path::new("/x/100%")), r#""/x/100%%""#);
        let entry = desktop_entry(Path::new("/usr/bin/nanomd"));
        assert!(entry.contains("Exec=\"/usr/bin/nanomd\" %f\n"), "{entry}");
        assert!(
            entry.contains("MimeType=text/markdown;text/x-markdown;\n"),
            "{entry}"
        );
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn unregistering_drops_only_nanomd_from_mimeapps() {
        let list = "[Default Applications]\n\
                    text/markdown=nanomd.desktop;\n\
                    text/x-markdown=other.desktop;nanomd.desktop;\n\
                    text/html=firefox.desktop\n\
                    [Added Associations]\n\
                    text/markdown=nanomd.desktop;gedit.desktop;\n";
        assert_eq!(
            super::imp::without(list, "nanomd.desktop"),
            "[Default Applications]\n\
             text/x-markdown=other.desktop;\n\
             text/html=firefox.desktop\n\
             [Added Associations]\n\
             text/markdown=gedit.desktop;\n"
        );
    }
}
