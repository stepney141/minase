//! debugging-tools.md「入出力のログの形式」を実行ファイルの境界で検査する。

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// テスト専用のディレクトリに置くログ。終了時にディレクトリごと削除する。
struct TemporaryLog(PathBuf);

impl TemporaryLog {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let directory = std::env::temp_dir().join(format!(
            "minase-io-log-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir(&directory).unwrap();
        Self(directory.join("session.log"))
    }
}

impl Drop for TemporaryLog {
    fn drop(&mut self) {
        fs::remove_dir_all(self.0.parent().unwrap()).unwrap();
    }
}

/// 子プロセスをテスト失敗時にも終了させる。
struct Session(Child);

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// 必須のプロトコルと規則を指定し、任意のログを設定する。
fn command(protocol: &str, log: Option<&Path>) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_minase"));
    command.args(["--protocol", protocol, "--rules", "engine-default"]);
    if let Some(log) = log {
        command.arg("--io-log").arg(log);
    }
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

/// 有限の入力列を渡し、標準出力をバイト列として回収する。
fn run(protocol: &str, log: Option<&Path>, input: &[u8]) -> Output {
    let mut child = command(protocol, log).spawn().unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    output
}

/// 全行の形式と時刻順を検査し、受信行と送信行を分けて返す。
fn records(path: &Path) -> (Vec<String>, Vec<String>) {
    let text = fs::read_to_string(path).unwrap();
    assert!(text.ends_with('\n'));
    let mut previous = 0;
    let mut received = Vec::new();
    let mut sent = Vec::new();
    for record in text.strip_suffix('\n').unwrap().split('\n') {
        let (time, rest) = record.split_once(' ').unwrap();
        assert!(!time.is_empty() && time.bytes().all(|byte| byte.is_ascii_digit()));
        let millis: u128 = time.parse().unwrap();
        assert!(millis >= previous, "{text}");
        previous = millis;
        let (direction, line) = rest.split_once(' ').unwrap();
        match direction {
            ">" => received.push(line.to_owned()),
            "<" => sent.push(line.to_owned()),
            _ => panic!("invalid direction: {record}"),
        }
    }
    (received, sent)
}

/// debugging-tools.md「入出力のログの形式」: 握手からbestmoveまで全送受信を時刻順に記録する。
#[test]
fn usi_session_records_handshake_search_and_exact_stdout() {
    let log = TemporaryLog::new();
    let mut session = Session(command("usi", Some(&log.0)).spawn().unwrap());
    let mut input = session.0.stdin.take().unwrap();
    let stdout = session.0.stdout.take().unwrap();
    let (sender, receiver) = mpsc::channel();
    let reader = thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            if sender.send(line).is_err() {
                break;
            }
        }
    });
    let commands = ["usi", "isready", "position startpos", "go depth 1"];
    for line in commands {
        writeln!(input, "{line}").unwrap();
    }
    input.flush().unwrap();
    let mut stdout = Vec::new();
    loop {
        let line = receiver
            .recv_timeout(Duration::from_secs(30))
            .expect("engine must respond within 30 seconds")
            .unwrap();
        let bestmove = line.starts_with("bestmove ");
        stdout.push(line);
        if bestmove {
            break;
        }
    }
    // プロセス終了前にも記録が見えることを、quitに先立つ入力行で検査する。
    let live_log = fs::read_to_string(&log.0).unwrap();
    assert!(live_log.contains(" > go depth 1\n"));
    writeln!(input, "quit").unwrap();
    drop(input);
    loop {
        match receiver.recv_timeout(Duration::from_secs(30)) {
            Ok(line) => stdout.push(line.unwrap()),
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(error) => panic!("engine did not close stdout: {error}"),
        }
    }
    assert!(session.0.wait().unwrap().success());
    reader.join().unwrap();
    let (received, sent) = records(&log.0);
    assert_eq!(received, [commands.as_slice(), &["quit"]].concat());
    assert!(sent.iter().any(|line| line == "usiok"));
    assert!(sent.iter().any(|line| line == "readyok"));
    assert!(sent.iter().any(|line| line.starts_with("bestmove ")));
    assert_eq!(sent, stdout);
}

/// debugging-tools.md「入出力のログの形式」: CRLFだけを取り除き、末尾の空白や単独のCRを保つ。
#[test]
fn input_preserves_whitespace_and_removes_only_line_endings() {
    let log = TemporaryLog::new();
    run(
        "usi",
        Some(&log.0),
        "usi  \t\r\nisready\r\n\r\n  未知の入力 \t\nquit\r".as_bytes(),
    );
    let (received, _) = records(&log.0);
    assert_eq!(
        received,
        ["usi  \t", "isready", "", "  未知の入力 \t", "quit\r"]
    );
}

/// debugging-tools.md「入出力のログの形式」: 既存ファイルを上書きせず終了コード1で拒否する。
#[test]
fn existing_log_is_preserved_and_startup_fails() {
    let log = TemporaryLog::new();
    let original = b"previous session\n\x00\xff";
    fs::write(&log.0, original).unwrap();
    let output = command("usi", Some(&log.0)).output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(!output.stderr.is_empty());
    assert!(output.stdout.is_empty());
    assert_eq!(fs::read(&log.0).unwrap(), original);
}

/// debugging-tools.md「入出力のログの形式」: ログの有無は決定的なセッションの標準出力を変えない。
#[test]
fn logging_preserves_stdout_byte_for_byte() {
    let log = TemporaryLog::new();
    let input = b"usi\nisready\nposition startpos\nmoves\nstate\nquit\n";
    let plain = run("usi", None, input);
    let logged = run("usi", Some(&log.0), input);
    assert_eq!(logged.stdout, plain.stdout);
    let (_, sent) = records(&log.0);
    assert_eq!(format!("{}\n", sent.join("\n")).as_bytes(), logged.stdout);
}

/// debugging-tools.md「入出力のログの形式」: CECPの握手も同じ形式で送受信を記録する。
#[test]
fn cecp_session_records_both_directions() {
    let log = TemporaryLog::new();
    let input = b"xboard\nprotover 2\nquit\n";
    let output = run("cecp", Some(&log.0), input);
    let (received, sent) = records(&log.0);
    assert_eq!(received, ["xboard", "protover 2", "quit"]);
    assert!(sent.iter().any(|line| line == "feature done=1"));
    assert_eq!(format!("{}\n", sent.join("\n")).as_bytes(), output.stdout);
}
