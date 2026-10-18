//! Strip ANSI escape sequences (CSI, OSC, and two-byte escapes) from output.

pub fn strip(input: &str) -> String {
    if !input.contains('\u{1b}') {
        return input.to_string();
    }
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        match chars.next() {
            // CSI: parameters/intermediates then one final byte in @..~
            Some('[') => {
                for n in chars.by_ref() {
                    if ('@'..='~').contains(&n) {
                        break;
                    }
                }
            }
            // OSC: terminated by BEL or ESC \
            Some(']') => {
                while let Some(n) = chars.next() {
                    if n == '\u{7}' {
                        break;
                    }
                    if n == '\u{1b}' && chars.peek() == Some(&'\\') {
                        chars.next();
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn strips_sequences() {
        assert_eq!(super::strip("\u{1b}[1;31mred\u{1b}[0m plain"), "red plain");
        assert_eq!(super::strip("\u{1b}]8;;http://x\u{7}link\u{1b}]8;;\u{7}"), "link");
        assert_eq!(super::strip("no escapes"), "no escapes");
    }
}
