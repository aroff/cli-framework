use cli_framework::security::sanitize_untrusted_output;

// AC1: ANSI CSI sequences stripped
#[test]
fn test_ansi_csi_stripped() {
    let input = "\x1b[31mred\x1b[0m";
    let output = sanitize_untrusted_output(input);
    assert_eq!(output, "red");
    assert!(
        !output.contains('\x1b'),
        "Output must not contain ESC bytes"
    );
}

// AC2: Control characters stripped
#[test]
fn test_control_chars_stripped() {
    let input = "\x00\x01\x02";
    let output = sanitize_untrusted_output(input);
    assert_eq!(output, "");
}

// AC3: UTF-8 multi-byte, newline, tab preserved
#[test]
fn test_utf8_newline_tab_preserved() {
    let input = "héllo\nworld\t!";
    let output = sanitize_untrusted_output(input);
    assert_eq!(output, "héllo\nworld\t!");
}

// Additional edge cases
#[test]
fn test_osc_sequence_stripped() {
    let input = "\x1b]0;title\x07normal";
    let output = sanitize_untrusted_output(input);
    assert_eq!(output, "normal");
    assert!(!output.contains('\x1b'));
}

#[test]
fn test_mixed_content() {
    let input = "hello \x1b[32mworld\x1b[0m\x00done";
    let output = sanitize_untrusted_output(input);
    assert_eq!(output, "hello worlddone");
}

#[test]
fn test_tab_preserved() {
    let output = sanitize_untrusted_output("\t");
    assert_eq!(output, "\t");
}

#[test]
fn test_newline_preserved() {
    let output = sanitize_untrusted_output("\n");
    assert_eq!(output, "\n");
}

#[test]
fn test_carriage_return_preserved() {
    let output = sanitize_untrusted_output("\r");
    assert_eq!(output, "\r");
}

#[test]
fn test_0x0b_stripped() {
    let input = "a\x0bb";
    let output = sanitize_untrusted_output(input);
    assert_eq!(output, "ab");
}

#[test]
fn test_plain_ascii_unchanged() {
    let input = "Hello, World!";
    let output = sanitize_untrusted_output(input);
    assert_eq!(output, "Hello, World!");
}

#[test]
fn test_sanitizer_removes_ansi_from_command_id() {
    let command_id = "\x1b[31mdrop_database\x1b[0m";
    let safe = sanitize_untrusted_output(command_id);
    assert_eq!(safe, "drop_database");
}

#[test]
fn test_sanitizer_removes_ansi_from_reasoning() {
    let reasoning = "This \x1b[1mcommand\x1b[0m is safe";
    let safe = sanitize_untrusted_output(reasoning);
    assert_eq!(safe, "This command is safe");
}

// --- AC4: display_resolution_to captures and asserts no ESC bytes in output ---

#[test]
fn test_ac4_display_resolution_sanitizes_command_id_and_reasoning() {
    use cli_framework::cli_output::display_resolution_to;
    use cli_framework::command::CommandArgs;
    use cli_framework::llm::CommandResolution;

    let resolution = CommandResolution {
        command_id: "\x1b[31mdrop_database\x1b[0m".to_string(),
        args: CommandArgs {
            positional: vec!["\x1b[32marg1\x1b[0m".to_string()],
            named: std::collections::HashMap::new(),
        },
        confidence: 0.9,
        reasoning: Some("\x1b[1mDangerous\x1b[0m operation".to_string()),
    };

    let mut output = Vec::<u8>::new();
    display_resolution_to(&resolution, &mut output);
    let text = String::from_utf8(output).unwrap();

    assert!(
        !text.contains('\x1b'),
        "Output must not contain ESC bytes; got: {:?}",
        text
    );
    assert!(
        text.contains("drop_database"),
        "Sanitized command_id should appear in output"
    );
    assert!(
        text.contains("Dangerous"),
        "Sanitized reasoning should appear in output"
    );
    assert!(
        text.contains("arg1"),
        "Sanitized positional arg should appear in output"
    );
}

// --- AC5: All eight display functions called with ANSI-injected inputs ---

#[test]
fn test_ac5_display_confirmation_sanitizes_ansi() {
    use cli_framework::cli_output::display_confirmation_to;
    use cli_framework::command::CommandArgs;
    use cli_framework::llm::CommandResolution;

    let mut named = std::collections::HashMap::new();
    named.insert(
        "\x1b[32mkey\x1b[0m".to_string(),
        "\x1b[33mvalue\x1b[0m".to_string(),
    );
    let resolution = CommandResolution {
        command_id: "\x1b[31mevil-cmd\x1b[0m".to_string(),
        args: CommandArgs {
            positional: vec![],
            named,
        },
        confidence: 0.8,
        reasoning: None,
    };

    let mut output = Vec::<u8>::new();
    display_confirmation_to(&resolution, Some("\x1b[35mctx\x1b[0m"), &mut output);
    let text = String::from_utf8(output).unwrap();

    assert!(
        !text.contains('\x1b'),
        "display_confirmation output must not contain ESC bytes; got: {:?}",
        text
    );
    assert!(text.contains("evil-cmd"));
    assert!(text.contains("key"));
    assert!(text.contains("value"));
    assert!(text.contains("ctx"));
}

#[test]
fn test_ac5_display_retry_sanitizes_ansi() {
    use cli_framework::cli_output::display_retry_to;

    let mut output = Vec::<u8>::new();
    display_retry_to(1, 3, "\x1b[31mconnection refused\x1b[0m", &mut output);
    let text = String::from_utf8(output).unwrap();

    assert!(
        !text.contains('\x1b'),
        "display_retry output must not contain ESC bytes; got: {:?}",
        text
    );
    assert!(text.contains("connection refused"));
}

#[test]
fn test_ac5_display_failure_sanitizes_ansi() {
    use cli_framework::cli_output::display_failure_to;

    let mut output = Vec::<u8>::new();
    display_failure_to(
        "\x1b[31mbad-cmd\x1b[0m",
        "\x1b[1mfatal error\x1b[0m",
        &mut output,
    );
    let text = String::from_utf8(output).unwrap();

    assert!(
        !text.contains('\x1b'),
        "display_failure output must not contain ESC bytes; got: {:?}",
        text
    );
    assert!(text.contains("bad-cmd"));
    assert!(text.contains("fatal error"));
}

#[test]
fn test_ac5_display_max_retries_exceeded_sanitizes_ansi() {
    use cli_framework::cli_output::display_max_retries_exceeded_to;

    let mut output = Vec::<u8>::new();
    display_max_retries_exceeded_to("\x1b[31mexhausted-cmd\x1b[0m", &mut output);
    let text = String::from_utf8(output).unwrap();

    assert!(
        !text.contains('\x1b'),
        "display_max_retries_exceeded output must not contain ESC bytes; got: {:?}",
        text
    );
    assert!(text.contains("exhausted-cmd"));
}

#[test]
fn test_ac5_display_success_sanitizes_ansi() {
    use cli_framework::cli_output::display_success_to;

    let mut output = Vec::<u8>::new();
    display_success_to("\x1b[32mgood-cmd\x1b[0m", &mut output);
    let text = String::from_utf8(output).unwrap();

    assert!(
        !text.contains('\x1b'),
        "display_success output must not contain ESC bytes; got: {:?}",
        text
    );
    assert!(text.contains("good-cmd"));
}

#[test]
fn test_ac5_display_suggestion_sanitizes_ansi() {
    use cli_framework::cli_output::display_suggestion_to;
    use cli_framework::command::CommandArgs;
    use cli_framework::llm::CommandResolution;

    let resolution = CommandResolution {
        command_id: "\x1b[33msuggested-cmd\x1b[0m".to_string(),
        args: CommandArgs::default(),
        confidence: 0.75,
        reasoning: Some("\x1b[1mbetter option\x1b[0m".to_string()),
    };

    let mut output = Vec::<u8>::new();
    display_suggestion_to(&resolution, &mut output);
    let text = String::from_utf8(output).unwrap();

    assert!(
        !text.contains('\x1b'),
        "display_suggestion output must not contain ESC bytes; got: {:?}",
        text
    );
    assert!(text.contains("suggested-cmd"));
    assert!(text.contains("better option"));
}

#[test]
fn test_ac5_display_command_help_sanitizes_ansi() {
    use cli_framework::cli_output::display_command_help_to;
    use cli_framework::llm::CommandMetadata;

    let commands = vec![CommandMetadata {
        id: "\x1b[31mevil-id\x1b[0m".to_string(),
        summary: "\x1b[32mevil summary\x1b[0m".to_string(),
        syntax: Some("\x1b[33mevil syntax\x1b[0m".to_string()),
        category: Some("general".to_string()),
    }];

    let mut output = Vec::<u8>::new();
    display_command_help_to(&commands, &mut output);
    let text = String::from_utf8(output).unwrap();

    assert!(
        !text.contains('\x1b'),
        "display_command_help output must not contain ESC bytes; got: {:?}",
        text
    );
    assert!(text.contains("evil-id"));
    assert!(text.contains("evil summary"));
    assert!(text.contains("evil syntax"));
}
