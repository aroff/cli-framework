use regex::Regex;
use std::sync::LazyLock;

// CSI: ESC [ <params> <final-byte>
static ANSI_CSI_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\x1b\[[0-9;]*[A-Za-z]").unwrap());

// OSC: ESC ] <data> (BEL | ESC \)
static ANSI_OSC_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\x1b\][^\x07\x1b]*([\x07]|\x1b\\)").unwrap());

/// Sanitize a string originating from an untrusted source (LLM, plugin, external API)
/// before printing to stdout or stderr.
///
/// Strips: control characters (except \n, \r, \t), ANSI CSI sequences, ANSI OSC sequences.
/// Preserves: printable ASCII, valid UTF-8 multi-byte characters, \n (0x0A), \r (0x0D), \t (0x09).
pub fn sanitize_untrusted_output(input: &str) -> String {
    // Step 1: strip ANSI CSI sequences before removing ESC so the regex can match them whole
    let step1 = ANSI_CSI_RE.replace_all(input, "");
    // Step 2: strip ANSI OSC sequences
    let step2 = ANSI_OSC_RE.replace_all(&step1, "");
    // Step 3: strip remaining control characters, preserving \t (0x09), \n (0x0A), \r (0x0D)
    step2
        .chars()
        .filter(|c| {
            let b = *c as u32;
            b > 0x1F || b == 0x09 || b == 0x0A || b == 0x0D
        })
        .collect()
}
