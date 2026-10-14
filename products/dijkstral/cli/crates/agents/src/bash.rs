//! Does a shell command line run a code-search program? (`docs/graphify-query.md` §11.3)
//!
//! A deliberately small lexer, not a shell parser: it finds words in command position (start of
//! the line, after `;`, `&`, `|`, a newline, `(`, `{`, `$(` or a backtick, after leading
//! `VAR=value` assignments, and after the wrappers in [`PREFIXES`] and their `-` options) and
//! compares their basename with [`SEARCH_PROGRAMS`]. Quoted words, arguments and redirect targets
//! are never commands, so `echo grep`, `bash -c 'rg x'` and `cat f>grep` stay silent.

/// Programs whose use means "searching the code". Matched case-insensitively on the basename,
/// with an optional `.exe`.
const SEARCH_PROGRAMS: &[&str] = &[
    "rg", "grep", "egrep", "fgrep", "zgrep", "ag", "ack", "fd", "find", "ripgrep",
];

/// Wrappers that run their first non-option argument as a command.
const PREFIXES: &[&str] = &[
    "sudo", "env", "time", "nice", "nohup", "exec", "command", "xargs",
];

/// `git` options that take the next word as their value.
const GIT_OPTIONS_WITH_VALUE: &[&str] = &[
    "-C",
    "-c",
    "--git-dir",
    "--work-tree",
    "--namespace",
    "--exec-path",
    "--config-env",
];

#[derive(Debug, PartialEq)]
enum Token {
    /// A word; `quoted` when it starts with a quote character.
    Word { text: String, quoted: bool },
    /// The next word is in command position.
    CommandStart,
    /// A group closed (`)`, `}`, closing backtick): what follows is not a command.
    GroupEnd,
}

/// True when `command` runs one of [`SEARCH_PROGRAMS`] (or `git grep`) in command position.
pub fn runs_search(command: &str) -> bool {
    #[derive(Clone, Copy)]
    enum State {
        /// Expecting a command; `wrapped` after a prefix such as `sudo`.
        Command {
            wrapped: bool,
        },
        /// Inside `git`'s global options; `skip` when the next word is an option value.
        Git {
            skip: bool,
        },
        Arguments,
    }
    let mut state = State::Command { wrapped: false };
    for token in lex(command) {
        state = match (token, state) {
            (Token::CommandStart, _) => State::Command { wrapped: false },
            (Token::GroupEnd, _) => State::Arguments,
            (Token::Word { .. }, State::Arguments) => State::Arguments,
            (Token::Word { .. }, State::Git { skip: true }) => State::Git { skip: false },
            (Token::Word { quoted: true, .. }, _) => State::Arguments,
            (Token::Word { text, .. }, State::Git { skip: false }) => {
                if text.starts_with('-') {
                    State::Git {
                        skip: GIT_OPTIONS_WITH_VALUE.contains(&text.as_str()),
                    }
                } else if text.eq_ignore_ascii_case("grep") {
                    return true;
                } else {
                    State::Arguments
                }
            }
            (Token::Word { text, .. }, State::Command { wrapped }) => {
                if is_assignment(&text) || (wrapped && text.starts_with('-')) {
                    State::Command { wrapped }
                } else {
                    let name = program_name(&text);
                    if SEARCH_PROGRAMS.contains(&name.as_str()) {
                        return true;
                    }
                    if PREFIXES.contains(&name.as_str()) {
                        State::Command { wrapped: true }
                    } else if name == "git" {
                        State::Git { skip: false }
                    } else {
                        State::Arguments
                    }
                }
            }
        };
    }
    false
}

/// Lower-cased basename without a trailing `.exe`.
fn program_name(word: &str) -> String {
    let base = word.rsplit('/').next().unwrap_or(word).to_ascii_lowercase();
    match base.strip_suffix(".exe") {
        Some(stem) => stem.to_string(),
        None => base,
    }
}

/// `NAME=...` with a shell identifier before the `=`.
fn is_assignment(word: &str) -> bool {
    let Some((name, _)) = word.split_once('=') else {
        return false;
    };
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Word accumulator for [`lex`].
#[derive(Default)]
struct Lexer {
    out: Vec<Token>,
    word: String,
    /// The current word started with a quote.
    quoted: bool,
    in_word: bool,
    /// The next word is a redirect target, not an argument or command: drop it.
    redirect_target: bool,
}

impl Lexer {
    fn push_char(&mut self, c: char) {
        self.word.push(c);
        self.in_word = true;
    }

    fn start_quote(&mut self) {
        self.quoted |= !self.in_word;
        self.in_word = true;
    }

    fn flush(&mut self) {
        if !self.in_word {
            return;
        }
        let text = std::mem::take(&mut self.word);
        if std::mem::take(&mut self.redirect_target) {
            // dropped
        } else {
            self.out.push(Token::Word {
                text,
                quoted: self.quoted,
            });
        }
        self.quoted = false;
        self.in_word = false;
    }

    fn token(&mut self, t: Token) {
        self.flush();
        self.out.push(t);
    }

    /// Discards a pending all-digit word (the file descriptor of `2>`); true if it did.
    fn take_fd(&mut self) -> bool {
        if self.in_word && !self.quoted && self.word.chars().all(|c| c.is_ascii_digit()) {
            self.word.clear();
            self.in_word = false;
            return true;
        }
        false
    }
}

fn lex(src: &str) -> Vec<Token> {
    let chars: Vec<char> = src.chars().collect();
    let mut lx = Lexer::default();
    let mut backticks_open = false;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        match c {
            ' ' | '\t' | '\r' => lx.flush(),
            '\\' => {
                if next == Some('\n') {
                    i += 1; // line continuation
                } else if let Some(n) = next {
                    lx.push_char(n);
                    i += 1;
                }
            }
            '\'' | '"' => {
                lx.start_quote();
                i += 1;
                while i < chars.len() && chars[i] != c {
                    if c == '"' && chars[i] == '\\' && i + 1 < chars.len() {
                        i += 1;
                    }
                    lx.word.push(chars[i]);
                    i += 1;
                }
            }
            '#' if !lx.in_word => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
                continue;
            }
            '$' if next == Some('{') => {
                // Parameter expansion stays inside the word.
                while i < chars.len() && chars[i] != '}' {
                    lx.push_char(chars[i]);
                    i += 1;
                }
                lx.push_char('}');
            }
            '$' if next == Some('(') => {
                lx.token(Token::CommandStart);
                i += 1;
            }
            '>' | '<' => {
                if !lx.take_fd() {
                    lx.flush();
                }
                while matches!(chars.get(i + 1), Some('>' | '<' | '&' | '|')) {
                    i += 1;
                }
                lx.redirect_target = true;
            }
            '&' if next == Some('>') => {
                lx.flush();
                i += 1;
                while matches!(chars.get(i + 1), Some('>' | '&')) {
                    i += 1;
                }
                lx.redirect_target = true;
            }
            ';' | '&' | '|' | '\n' | '(' => {
                lx.token(Token::CommandStart);
                lx.redirect_target = false;
            }
            '{' if !lx.in_word => lx.token(Token::CommandStart),
            '}' if !lx.in_word => lx.token(Token::GroupEnd),
            ')' => lx.token(Token::GroupEnd),
            '`' => {
                backticks_open = !backticks_open;
                lx.token(if backticks_open {
                    Token::CommandStart
                } else {
                    Token::GroupEnd
                });
            }
            _ => lx.push_char(c),
        }
        i += 1;
    }
    lx.flush();
    lx.out
}

#[cfg(test)]
mod tests {
    use super::runs_search;

    #[test]
    fn search_programs_in_command_position_match() {
        for cmd in [
            "rg foo",
            "grep -rn foo .",
            "egrep x",
            "fgrep x",
            "zgrep x f.gz",
            "ag x",
            "ack x",
            "fd x",
            "find . -name '*.py'",
            "ripgrep x",
            "git grep x",
            "git -C . grep x",
            "git  grep x",
            "git -c core.x=1 grep x",
            "rg",
            "find",
            "fd",
            "grep",
            "git grep",
            "rg --files",
            "RG x",
            "Grep x",
            "  rg x",
            "cd src; rg x",
            "cd src;rg x",
            "true && rg x",
            "true&&rg x",
            "false || grep x",
            "cat f | grep x",
            "cat f|grep x",
            "sleep 1 & rg x",
            "echo a\nrg x",
            "(rg x)",
            "{ rg x; }",
            "echo $(rg x)",
            "echo `rg x`",
            "sudo rg x",
            "env rg x",
            "env FOO=1 rg x",
            "time rg x",
            "nice rg x",
            "nohup rg x",
            "exec rg x",
            "command rg x",
            "xargs grep x",
            "ls | xargs -0 grep x",
            "FOO=1 rg x",
            "A=1 B=2 grep x",
            "/usr/bin/grep x",
            "./rg x",
            "rg.exe x",
            "find.exe .",
            "graphify query x && rg x",
            "dijkstral query x && rg x",
            "rg x dijkstral-out/",
            "cmd 2>&1 | grep x",
            ">out rg x",
        ] {
            assert!(runs_search(cmd), "should match: {cmd:?}");
        }
    }

    #[test]
    fn other_commands_and_non_command_positions_do_not_match() {
        for cmd in [
            "",
            "ugrep x",
            "rga x",
            "locate x",
            "pgrep x",
            "findstr x",
            "fd-find x",
            "ast-grep x",
            "sg x",
            "semgrep x",
            "sk",
            "fzf",
            "tree",
            "ls -R",
            "cat f",
            "head f",
            "bat f",
            "sed -n 1p f",
            "awk '{print}' f",
            "git log --grep x",
            "git ls-files",
            "git status",
            "git diff",
            "npm test",
            "cargo test",
            "rgx",
            "rg-x",
            "grepx",
            "mygrep",
            "echo grep",
            "echo find",
            "echo fd x",
            "echo 'rg x'",
            "bash -c 'rg x'",
            "\"rg\" x",
            "'grep' x",
            "x=grep",
            "rg='x'",
            "cat f>grep",
            "watch rg x",
            "timeout 5 rg x",
            "sudo -u x rg y",
            "! rg x",
            "if rg x",
            "graphify query x",
            "dijkstral query \"where is grep used\"",
            "echo ${x}rg",
            "echo hi # ; rg x",
            "cmd 2>&1",
            "cmd &>log",
        ] {
            assert!(!runs_search(cmd), "should not match: {cmd:?}");
        }
    }
}
