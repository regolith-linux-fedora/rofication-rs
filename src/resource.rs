//! Resource lookups: an environment variable, else `xrescat`/`trawlcat`
//! (`trawlcat` first on Wayland), else the default.

use std::borrow::Cow;
use std::env;
use std::process::{Command, Stdio};

use crate::pystr;

#[derive(Clone, Debug)]
pub struct Resource {
    pub default: Cow<'static, str>,
    pub xres_name: Cow<'static, str>,
    pub env_name: Option<Cow<'static, str>>,
}

impl Resource {
    pub const fn new(
        env_name: &'static str,
        xres_name: &'static str,
        default: &'static str,
    ) -> Self {
        Resource {
            default: Cow::Borrowed(default),
            xres_name: Cow::Borrowed(xres_name),
            env_name: Some(Cow::Borrowed(env_name)),
        }
    }

    /// A resource without an environment variable.
    pub const fn xres_only(xres_name: &'static str, default: &'static str) -> Self {
        Resource {
            default: Cow::Borrowed(default),
            xres_name: Cow::Borrowed(xres_name),
            env_name: None,
        }
    }

    /// The value as the source returns it (no trimming). A getter that is
    /// missing, cannot run or exits non-zero is skipped; output that is not
    /// UTF-8 is fatal, as it was for the original.
    pub fn fetch(&self) -> String {
        if let Some(name) = &self.env_name {
            if let Some(value) = env::var_os(name.as_ref()) {
                // Avoid calling a resource getter when the variable is set.
                return value.to_string_lossy().into_owned();
            }
        }
        let wayland = env::var_os("XDG_SESSION_TYPE").is_some_and(|v| v == "wayland");
        let getters: [&str; 2] = if wayland {
            ["trawlcat", "xrescat"]
        } else {
            ["xrescat", "trawlcat"]
        };
        for getter in getters {
            let output = Command::new(getter)
                .arg(self.xres_name.as_ref())
                .arg(self.default.as_ref())
                .stdin(Stdio::inherit())
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit())
                .output();
            let Ok(output) = output else {
                continue;
            };
            let Ok(text) = String::from_utf8(output.stdout) else {
                crate::die(&format!(
                    "UnicodeDecodeError: {getter} printed a value that is not UTF-8"
                ));
            };
            if !output.status.success() {
                continue;
            }
            return pystr::translate_newlines(&text);
        }
        self.default.to_string()
    }
}
