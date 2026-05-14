use std::{
   io::Write as _,
   process::{
      Command,
      Stdio,
   },
   time::{
      Duration,
      Instant,
   },
};

const TIMEOUT: Duration = Duration::from_secs(1);

type CmdEntry = (&'static str, &'static [&'static str]);

// Try every backend in order; spawn failures fall through to the next one. The
// missing-binary case fails fast (Command::spawn returns NotFound) so this
// costs nothing when only one of these is actually installed.
#[cfg(target_os = "linux")]
const COMMANDS: &[CmdEntry] = &[
   ("wl-copy", &[]),
   ("xsel", &["-i", "-b"]),
   ("xclip", &["-selection", "clipboard"]),
];

#[cfg(target_os = "macos")]
const COMMANDS: &[CmdEntry] = &[("pbcopy", &[])];

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
const COMMANDS: &[CmdEntry] = &[];

pub fn copy(text: &str) -> Result<(), Vec<String>> {
   if COMMANDS.is_empty() {
      return Err(vec![
         "no clipboard backend available on this platform".to_string(),
      ]);
   }
   let mut errs = Vec::new();
   for (cmd, args) in COMMANDS {
      match run_one(cmd, args, text) {
         Ok(()) => return Ok(()),
         Err(e) => errs.push(format!("Running ({cmd:?}, {args:?}) {e}.")),
      }
   }
   Err(errs)
}

fn run_one(cmd: &str, args: &[&str], text: &str) -> Result<(), String> {
   let mut child = Command::new(cmd)
      .args(args)
      .stdin(Stdio::piped())
      .stdout(Stdio::null())
      .stderr(Stdio::null())
      .spawn()
      .map_err(|e| format!("failed with exception: {e}"))?;

   if let Some(mut stdin) = child.stdin.take() {
      stdin
         .write_all(text.as_bytes())
         .map_err(|e| format!("failed with exception: {e}"))?;
   }

   let start = Instant::now();
   loop {
      match child.try_wait() {
         Ok(Some(status)) if status.success() => return Ok(()),
         Ok(Some(status)) => {
            return Err(format!(
               "failed with exit code {}",
               status.code().unwrap_or(-1)
            ));
         },
         Ok(None) => {
            if start.elapsed() >= TIMEOUT {
               let _ = child.kill();
               return Err("timed out".to_string());
            }
            std::thread::sleep(Duration::from_millis(20));
         },
         Err(e) => return Err(format!("failed with exception: {e}")),
      }
   }
}
