/// Safely truncate a string at a byte boundary, respecting multi-byte UTF-8.
pub(crate) fn truncate_str(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Render content with line numbers prepended to each line.
///
/// Each line is prefixed with its absolute line number starting from `start_line`,
/// in the format "{n}: {line}". Lines are rejoined with `\n`.
///
/// Trailing-newline semantics: this is a faithful, round-trippable transform over
/// `split('\n')`, so a `content` that ends in `\n` yields a final numbered empty
/// segment (e.g. `"a\n"` at line 5 → `"5: a\n6: "`). This differs from `grep -n`,
/// which emits nothing after a trailing newline. The faithful form is intentional
/// for snippet display, where the input may be a byte-truncated fragment.
///
/// Numbering saturates at `u32::MAX` rather than overflowing: pathological inputs
/// (a near-`u32::MAX` `start_line`, or more lines than `u32::MAX`) clamp the number
/// instead of panicking in debug or wrapping in release.
pub(crate) fn render_with_line_numbers(content: &str, start_line: u32) -> String {
    content
        .split('\n')
        .enumerate()
        .map(|(i, line)| {
            let n = start_line.saturating_add(u32::try_from(i).unwrap_or(u32::MAX));
            format!("{n}: {line}")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_with_line_numbers_multi_line() {
        let content = "first\nsecond\nthird";
        let result = render_with_line_numbers(content, 12);
        assert_eq!(result, "12: first\n13: second\n14: third");
    }

    #[test]
    fn test_render_with_line_numbers_single_line() {
        let content = "single line";
        let result = render_with_line_numbers(content, 42);
        assert_eq!(result, "42: single line");
    }

    #[test]
    fn test_render_with_line_numbers_empty() {
        let content = "";
        let result = render_with_line_numbers(content, 1);
        assert_eq!(result, "1: ");
    }

    #[test]
    fn test_render_with_line_numbers_trailing_newline() {
        let content = "line one\nline two\n";
        let result = render_with_line_numbers(content, 5);
        assert_eq!(result, "5: line one\n6: line two\n7: ");
    }

    #[test]
    fn test_render_with_line_numbers_start_line_one() {
        let content = "alpha\nbeta";
        let result = render_with_line_numbers(content, 1);
        assert_eq!(result, "1: alpha\n2: beta");
    }

    #[test]
    fn test_render_with_line_numbers_start_line_zero() {
        let content = "first\nsecond";
        let result = render_with_line_numbers(content, 0);
        assert_eq!(result, "0: first\n1: second");
    }

    #[test]
    fn test_render_with_line_numbers_saturates_near_u32_max() {
        let content = "first\nsecond\nthird";
        let result = render_with_line_numbers(content, u32::MAX - 1);
        // Numbers clamp at u32::MAX instead of wrapping/panicking.
        assert_eq!(
            result,
            format!(
                "{}: first\n{}: second\n{}: third",
                u32::MAX - 1,
                u32::MAX,
                u32::MAX
            )
        );
    }
}
