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
    // OSC sequence: ESC ] ... BEL
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
    // U+000B (vertical tab) should be stripped
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

// AC4/AC5: Verify sanitization is called in print_resolution-like scenarios
// (we test the sanitizer directly; integration of call sites is verified by code inspection)
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
