#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchMode {
    Passthrough,
    Compositor,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlshOptions {
    pub predict: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedSshArgs {
    pub forwarded_args: Vec<String>,
    pub ssh_options: Vec<String>,
    pub host: Option<String>,
    pub remote_command: Vec<String>,
    pub slsh: SlshOptions,
    pub mode: LaunchMode,
}

#[derive(Debug, Default)]
struct ParseState {
    ssh_options: Vec<String>,
    host: Option<String>,
    remote_command: Vec<String>,
    no_session: bool,
    stdio_detached: bool,
    print_only: bool,
    subsystem: bool,
    disable_tty: bool,
    force_tty: bool,
}

pub fn parse(args: Vec<String>, stdin_is_tty: bool, stdout_is_tty: bool) -> ParsedSshArgs {
    let mut forwarded_args = Vec::new();
    let mut slsh = SlshOptions { predict: true };

    for arg in args {
        if arg == "--slsh-no-predict" {
            slsh.predict = false;
        } else {
            forwarded_args.push(arg);
        }
    }

    let state = inspect_forwarded(&forwarded_args);
    let mode = if !stdin_is_tty
        || !stdout_is_tty
        || state.host.is_none()
        || state.no_session
        || state.stdio_detached
        || state.print_only
        || state.subsystem
        || (state.disable_tty && !state.force_tty)
    {
        LaunchMode::Passthrough
    } else {
        LaunchMode::Compositor
    };

    ParsedSshArgs {
        forwarded_args,
        ssh_options: state.ssh_options,
        host: state.host,
        remote_command: state.remote_command,
        slsh,
        mode,
    }
}

fn inspect_forwarded(args: &[String]) -> ParseState {
    let mut state = ParseState::default();
    let mut i = 0;
    let mut host_seen = false;

    while i < args.len() {
        let arg = &args[i];

        if host_seen {
            state.remote_command.push(arg.clone());
            i += 1;
            continue;
        }

        if arg == "--" {
            if let Some(host) = args.get(i + 1) {
                state.host = Some(host.clone());
                state.remote_command.extend(args[i + 2..].iter().cloned());
            }
            break;
        }

        if !arg.starts_with('-') || arg == "-" {
            state.host = Some(arg.clone());
            host_seen = true;
            i += 1;
            continue;
        }

        let consumed = inspect_option(args, i, &mut state);
        let end = (i + consumed.max(1)).min(args.len());
        state.ssh_options.extend(args[i..end].iter().cloned());
        i += consumed.max(1);
    }

    state
}

fn inspect_option(args: &[String], index: usize, state: &mut ParseState) -> usize {
    let arg = &args[index];

    if arg == "-N" {
        state.no_session = true;
        return 1;
    }
    if arg == "-G" || arg == "-V" {
        state.print_only = true;
        return 1;
    }
    if arg == "-T" {
        state.disable_tty = true;
        return 1;
    }
    if arg == "-s" {
        state.subsystem = true;
        return 1;
    }
    if arg == "-n" || arg == "-f" {
        state.stdio_detached = true;
        return 1;
    }
    if arg == "-t" || arg == "-tt" {
        state.force_tty = true;
        return 1;
    }
    if arg.starts_with("-t") && arg.chars().skip(1).all(|ch| ch == 't') {
        state.force_tty = true;
        return 1;
    }

    if let Some(short) = single_short_option(arg) {
        if option_takes_arg(short) {
            mark_arg_option(short, state);
            return if arg.len() > 2 { 1 } else { 2 };
        }

        mark_flag_group(&arg[1..], state);
        return 1;
    }

    if arg.starts_with("-o") && arg.len() > 2 {
        return 1;
    }

    1
}

fn single_short_option(arg: &str) -> Option<char> {
    let mut chars = arg.chars();
    if chars.next() != Some('-') {
        return None;
    }
    chars.next()
}

fn option_takes_arg(short: char) -> bool {
    matches!(
        short,
        'B' | 'b'
            | 'c'
            | 'D'
            | 'E'
            | 'e'
            | 'F'
            | 'I'
            | 'i'
            | 'J'
            | 'L'
            | 'l'
            | 'm'
            | 'O'
            | 'o'
            | 'p'
            | 'Q'
            | 'R'
            | 'S'
            | 'W'
            | 'w'
    )
}

fn mark_arg_option(short: char, state: &mut ParseState) {
    match short {
        'G' | 'Q' | 'V' => state.print_only = true,
        'O' | 'W' => state.no_session = true,
        _ => {}
    }
}

fn mark_flag_group(flags: &str, state: &mut ParseState) {
    for ch in flags.chars() {
        match ch {
            'N' => state.no_session = true,
            'G' | 'V' => state.print_only = true,
            'T' => state.disable_tty = true,
            't' => state.force_tty = true,
            's' => state.subsystem = true,
            'n' | 'f' => state.stdio_detached = true,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_tty(args: &[&str]) -> ParsedSshArgs {
        parse(args.iter().map(|arg| arg.to_string()).collect(), true, true)
    }

    #[test]
    fn parses_basic_host_as_compositor() {
        let parsed = parse_tty(&["user@host"]);

        assert_eq!(parsed.host.as_deref(), Some("user@host"));
        assert_eq!(parsed.remote_command, Vec::<String>::new());
        assert_eq!(parsed.ssh_options, Vec::<String>::new());
        assert_eq!(parsed.mode, LaunchMode::Compositor);
    }

    #[test]
    fn preserves_common_options() {
        let parsed = parse_tty(&[
            "-p2222",
            "-i",
            "key",
            "-oStrictHostKeyChecking=no",
            "-J",
            "jump",
            "user@host",
        ]);

        assert_eq!(parsed.host.as_deref(), Some("user@host"));
        assert_eq!(parsed.mode, LaunchMode::Compositor);
        assert_eq!(
            parsed.ssh_options,
            vec![
                "-p2222",
                "-i",
                "key",
                "-oStrictHostKeyChecking=no",
                "-J",
                "jump",
            ]
        );
        assert_eq!(
            parsed.forwarded_args,
            vec![
                "-p2222",
                "-i",
                "key",
                "-oStrictHostKeyChecking=no",
                "-J",
                "jump",
                "user@host",
            ]
        );
    }

    #[test]
    fn keeps_forwarding_interactive() {
        let parsed = parse_tty(&["-L", "8080:localhost:80", "user@host"]);

        assert_eq!(parsed.host.as_deref(), Some("user@host"));
        assert_eq!(parsed.mode, LaunchMode::Compositor);
    }

    #[test]
    fn remote_command_stays_interactive() {
        let parsed = parse_tty(&["user@host", "htop"]);

        assert_eq!(parsed.host.as_deref(), Some("user@host"));
        assert_eq!(parsed.remote_command, vec!["htop"]);
        assert_eq!(parsed.mode, LaunchMode::Compositor);
    }

    #[test]
    fn strips_slsh_flag() {
        let parsed = parse_tty(&["--slsh-no-predict", "user@host"]);

        assert!(!parsed.slsh.predict);
        assert_eq!(parsed.forwarded_args, vec!["user@host"]);
    }

    #[test]
    fn non_tty_stdio_falls_back() {
        let parsed = parse(vec!["user@host".into()], false, true);

        assert_eq!(parsed.mode, LaunchMode::Passthrough);
    }

    #[test]
    fn noninteractive_flags_fall_back() {
        for args in [
            vec!["-N", "-L", "8080:localhost:80", "user@host"],
            vec!["-W", "other:22", "jump"],
            vec!["-G", "user@host"],
            vec!["-O", "check", "user@host"],
            vec!["-T", "user@host"],
            vec!["-n", "user@host"],
            vec!["-f", "user@host"],
            vec!["-Q", "cipher"],
            vec!["-V"],
            vec!["-s", "user@host", "sftp"],
        ] {
            assert_eq!(
                parse_tty(&args).mode,
                LaunchMode::Passthrough,
                "{args:?} should fall back"
            );
        }
    }

    #[test]
    fn explicit_tty_beats_disable_tty() {
        let parsed = parse_tty(&["-T", "-t", "user@host", "python3"]);

        assert_eq!(parsed.mode, LaunchMode::Compositor);
    }

    #[test]
    fn double_dash_keeps_host_boundary() {
        let parsed = parse_tty(&["-p", "2222", "--", "user@host", "echo", "hi"]);

        assert_eq!(parsed.host.as_deref(), Some("user@host"));
        assert_eq!(parsed.remote_command, vec!["echo", "hi"]);
        assert_eq!(parsed.mode, LaunchMode::Compositor);
    }
}
