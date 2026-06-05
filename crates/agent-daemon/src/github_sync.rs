/// Escapes double quotes in the input string.
fn esc(input: &str) -> String {
    input.replace("\"", "\\\"")
}