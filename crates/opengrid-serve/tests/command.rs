//! The command itself: `propose`, then `--config` on a file nobody has read.

mod support;

use std::process::Command;

use support::{Database, SQLITE_TABLES};

fn command() -> Command {
    Command::new(env!("CARGO_BIN_EXE_opengrid-server"))
}

#[test]
fn propose_writes_a_file_and_the_file_does_not_start_until_it_is_read() {
    let database = Database::new("command", SQLITE_TABLES);
    let out = std::env::temp_dir().join(format!("opengrid-serve-{}-out.json", std::process::id()));
    let _ = std::fs::remove_file(&out);

    let done = command()
        .args(["propose", "--out"])
        .arg(&out)
        .env("OPENGRID_DATABASE_URL", database.connection())
        .output()
        .expect("runs");
    assert!(
        done.status.success(),
        "{}",
        String::from_utf8_lossy(&done.stderr)
    );
    let written = std::fs::read_to_string(&out).expect("the proposal");
    assert!(written.contains("\"proposal\": true"));
    assert!(
        !written.contains(&database.connection()),
        "the connection string is never written"
    );

    // The owner's file is never overwritten.
    let again = command()
        .args(["propose", "--out"])
        .arg(&out)
        .env("OPENGRID_DATABASE_URL", database.connection())
        .output()
        .expect("runs");
    assert!(!again.status.success());
    assert_eq!(std::fs::read_to_string(&out).unwrap(), written);

    // A proposal is not served.
    let mut served = command();
    served
        .arg("--config")
        .arg(&out)
        .env("OPENGRID_DATABASE_URL", database.connection())
        .env(
            "OPENGRID_TOKEN",
            "a-random-secret-of-at-least-32-characters",
        );
    let served = finishes(&mut served);
    assert!(!served.status.success());
    assert!(
        String::from_utf8_lossy(&served.stderr).contains("still a proposal"),
        "{}",
        String::from_utf8_lossy(&served.stderr)
    );
    let _ = std::fs::remove_file(&out);
}

/// Runs `command` and waits for it to end: a server that did start (and is
/// not supposed to) is killed after a few seconds and fails the test, instead
/// of hanging it.
fn finishes(command: &mut Command) -> std::process::Output {
    use std::process::Stdio;
    use std::time::{Duration, Instant};

    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("starts");
    let deadline = Instant::now() + Duration::from_secs(15);
    while child.try_wait().expect("waits").is_none() {
        if Instant::now() > deadline {
            let _ = child.kill();
            panic!("the command did not end: it started serving");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    child.wait_with_output().expect("output")
}
