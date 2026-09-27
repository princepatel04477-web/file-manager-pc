pub const SESSION_LIFETIME_SECONDS: u64 = 10 * 60;
pub mod pc;

pub fn safe_attachment_name(name: &str) -> String {
    let value = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_' | ' ') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    if value.is_empty() { "download".to_owned() } else { value }
}
