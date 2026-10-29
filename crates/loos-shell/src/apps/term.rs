//! LoCmd: a cmd.exe-style command interpreter. Accepts LoOS paths and Windows paths
//! (`C:\Users\user` -> `/home/user`, via loos-vfs::winpath).

use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// What the terminal needs from the system.
pub trait Host {
    /// Directory listing: (name, is_dir, size); `None` if not a directory.
    fn list(&self, path: &str) -> Option<Vec<(String, bool, u64)>>;
    /// File contents; `None` if not a file.
    fn read(&self, path: &str) -> Option<Vec<u8>>;
    /// Create/replace a file.
    fn write(&self, path: &str, data: &[u8]) -> bool;
    /// Create a directory.
    fn mkdir(&self, path: &str) -> bool;
    /// Delete a file or directory tree.
    fn remove(&self, path: &str) -> bool;
    /// "pid name" lines.
    fn processes(&self) -> String;
}

/// Terminal state: scrollback, input line, history, working directory.
pub struct Term {
    /// Output lines.
    pub out: Vec<String>,
    /// Current input.
    pub input: String,
    /// Working directory (LoOS path).
    pub cwd: String,
    /// Previous commands.
    pub history: Vec<String>,
    hist_pos: usize,
    /// Set by `exit`.
    pub exit: bool,
    /// Window title set by `title`.
    pub title: Option<String>,
}

const MAX_LINES: usize = 500;
const HELP: &str = "CD/CHDIR  change directory     CLS       clear the screen
DIR/LS    list a directory     ECHO      print text (ECHO text > file)
TYPE/CAT  show a file          MKDIR/MD  make a directory
DEL/RM    delete               TASKLIST  running processes
VER       version              WHOAMI    current user
TITLE     window title         EXIT      close the window
HISTORY   previous commands    Paths: /home/user or C:\\Users\\user";

impl Term {
    /// New terminal in `home`.
    pub fn new(home: &str) -> Self {
        let mut t = Term { out: Vec::new(), input: String::new(), cwd: home.into(), history: Vec::new(), hist_pos: 0, exit: false, title: None };
        t.print("LoOS LoCmd 1.0 - type HELP for commands. Local only: nothing here talks to the network.");
        t.print("");
        t
    }
    /// Prompt text.
    pub fn prompt(&self) -> String {
        alloc::format!("{}> ", self.cwd)
    }
    fn print(&mut self, s: &str) {
        for l in s.split('\n') {
            self.out.push(l.to_string());
        }
        let n = self.out.len();
        if n > MAX_LINES {
            self.out.drain(..n - MAX_LINES);
        }
    }
    /// Resolves a user path (relative, absolute, or Windows) to a normalized LoOS path.
    pub fn resolve(&self, p: &str) -> String {
        let p = p.trim().trim_matches('"');
        let base = if p.len() >= 2 && p.as_bytes()[1] == b':' {
            loos_vfs::winpath::to_loos(p).unwrap_or_else(|| p.into())
        } else if p.starts_with('/') {
            p.into()
        } else {
            alloc::format!("{}/{}", self.cwd, p)
        };
        let mut parts: Vec<&str> = Vec::new();
        for c in base.split(['/', '\\']) {
            match c {
                "" | "." => {}
                ".." => {
                    parts.pop();
                }
                c => parts.push(c),
            }
        }
        if parts.is_empty() { "/".into() } else { parts.iter().fold(String::new(), |a, c| a + "/" + c) }
    }
    /// History navigation (Up = -1, Down = +1).
    pub fn recall(&mut self, dir: i32) {
        if self.history.is_empty() {
            return;
        }
        let n = self.history.len();
        self.hist_pos = if dir < 0 { self.hist_pos.saturating_sub(1) } else { (self.hist_pos + 1).min(n) };
        self.input = self.history.get(self.hist_pos).cloned().unwrap_or_default();
    }
    /// Runs the current input line.
    pub fn enter(&mut self, host: &dyn Host) {
        let line = core::mem::take(&mut self.input);
        let shown = alloc::format!("{}{}", self.prompt(), line);
        self.print(&shown);
        let line = line.trim();
        if !line.is_empty() {
            self.history.push(line.into());
        }
        self.hist_pos = self.history.len();
        self.run(line, host);
    }

    fn run(&mut self, line: &str, host: &dyn Host) {
        let (cmd, arg) = match line.split_once(char::is_whitespace) {
            Some((c, a)) => (c, a.trim()),
            None => (line, ""),
        };
        match cmd.to_ascii_lowercase().as_str() {
            "" => {}
            "help" | "?" => self.print(HELP),
            "cls" | "clear" => self.out.clear(),
            "ver" => self.print("LoOS [Version 0.1] - LoCmd"),
            "whoami" => self.print("loos\\user"),
            "exit" => self.exit = true,
            "title" => self.title = Some(arg.into()),
            "history" => {
                let h = self.history.join("\n");
                self.print(&h);
            }
            "echo" => match arg.split_once('>') {
                Some((text, file)) => {
                    let p = self.resolve(file);
                    let mut data = text.trim_end().as_bytes().to_vec();
                    data.push(b'\n');
                    if !host.write(&p, &data) {
                        self.print("The system cannot write the file.");
                    }
                }
                None => self.print(if arg.is_empty() { "ECHO is on." } else { arg }),
            },
            "cd" | "chdir" => {
                if arg.is_empty() {
                    let c = self.cwd.clone();
                    self.print(&c);
                } else {
                    let p = self.resolve(arg);
                    if host.list(&p).is_some() { self.cwd = p } else { self.print("The system cannot find the path specified.") }
                }
            }
            "dir" | "ls" => {
                let p = if arg.is_empty() { self.cwd.clone() } else { self.resolve(arg) };
                match host.list(&p) {
                    Some(items) => {
                        self.print(&alloc::format!(" Directory of {}", p));
                        self.print("");
                        let (mut files, mut dirs, mut bytes) = (0, 0, 0u64);
                        for (name, dir, size) in &items {
                            let l = if *dir { alloc::format!("    <DIR>          {}", name) } else { alloc::format!("    {:>14} {}", size, name) };
                            self.print(&l);
                            if *dir { dirs += 1 } else { files += 1; bytes += size }
                        }
                        self.print(&alloc::format!("{:>12} File(s) {:>14} bytes\n{:>12} Dir(s)", files, bytes, dirs));
                    }
                    None => self.print("File Not Found"),
                }
            }
            "type" | "cat" => {
                let p = self.resolve(arg);
                match host.read(&p) {
                    Some(d) => {
                        let s = String::from_utf8_lossy(&d).into_owned();
                        self.print(s.trim_end_matches('\n'));
                    }
                    None => self.print("The system cannot find the file specified."),
                }
            }
            "mkdir" | "md" => {
                let p = self.resolve(arg);
                if arg.is_empty() || !host.mkdir(&p) {
                    self.print("A subdirectory or file could not be created.");
                }
            }
            "del" | "rm" | "erase" | "rd" | "rmdir" => {
                let p = self.resolve(arg);
                if arg.is_empty() || !host.remove(&p) {
                    self.print("Could Not Find the item.");
                }
            }
            "tasklist" | "ps" => {
                let s = host.processes();
                self.print("  PID Image Name\n===== ====================");
                for l in s.lines() {
                    if let Some((pid, name)) = l.split_once(' ') {
                        self.print(&alloc::format!("{:>5} {}", pid, name));
                    }
                }
            }
            _ => self.print(&alloc::format!("'{}' is not recognized as a command. Type HELP for a list.", cmd)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::cell::RefCell;

    struct Fake(RefCell<Vec<(String, Vec<u8>)>>);
    impl Host for Fake {
        fn list(&self, p: &str) -> Option<Vec<(String, bool, u64)>> {
            (p == "/home/user" || p == "/home").then(|| alloc::vec![("Documents".into(), true, 0)])
        }
        fn read(&self, p: &str) -> Option<Vec<u8>> {
            self.0.borrow().iter().find(|f| f.0 == p).map(|f| f.1.clone())
        }
        fn write(&self, p: &str, d: &[u8]) -> bool {
            self.0.borrow_mut().push((p.into(), d.into()));
            true
        }
        fn mkdir(&self, _: &str) -> bool { true }
        fn remove(&self, _: &str) -> bool { false }
        fn processes(&self) -> String { "1 system\n4 loos-shell\n".into() }
    }

    fn run(t: &mut Term, h: &Fake, s: &str) -> String {
        t.out.clear();
        t.input = s.into();
        t.enter(h);
        t.out[1..].join("\n")
    }

    #[test]
    fn commands() {
        let h = Fake(RefCell::new(Vec::new()));
        let mut t = Term::new("/home/user");
        assert!(run(&mut t, &h, "dir").contains("<DIR>          Documents"));
        run(&mut t, &h, "echo hello > note.txt");
        assert_eq!(run(&mut t, &h, "type C:\\Users\\user\\note.txt"), "hello");
        run(&mut t, &h, "cd ..");
        assert_eq!(t.cwd, "/home");
        assert!(run(&mut t, &h, "cd /nope").contains("cannot find"));
        assert!(run(&mut t, &h, "tasklist").contains("    4 loos-shell"));
        assert!(run(&mut t, &h, "frob").contains("not recognized"));
        run(&mut t, &h, "exit");
        assert!(t.exit);
        t.recall(-1);
        assert_eq!(t.input, "exit");
    }
}
