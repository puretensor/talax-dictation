//! Run only inside an isolated X server without a clipboard manager:
//! `xvfb-run -a cargo test -p talax-engine --test clipboard_x11 -- --ignored`
#![cfg(target_os = "linux")]

use std::process::Command;
use talax_engine::inject::{InjectionConfig, TextInjector, release_clipboard};

#[test]
#[ignore = "requires an isolated X11 server and xclip"]
fn clipboard_survives_temporary_injector_and_can_be_replaced() {
    for text in [
        "first synthetic transcript",
        "replacement synthetic transcript",
    ] {
        // Match the app's temporary injector lifetime. No other Clipboard
        // object may keep arboard's in-process owner alive during this write.
        TextInjector::new(InjectionConfig::default())
            .inject(text)
            .unwrap();

        let output = Command::new("timeout")
            .args(["5s", "xclip", "-selection", "clipboard", "-out"])
            .output()
            .expect("xclip must be installed for the isolated X11 regression");
        assert!(
            output.status.success(),
            "clipboard read failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(String::from_utf8(output.stdout).unwrap(), text);
    }
    release_clipboard();
}
