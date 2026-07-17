/// Parse the account identifier shape observed in CN 3.8 SDK requests.
///
/// The client-provided value is a positive decimal user ID. Channel prefixes
/// are not accepted because the current APK and request evidence does not
/// establish a prefix contract.
pub fn parse_user_id(account_id: &str) -> Option<i64> {
    if account_id.is_empty() || !account_id.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }

    account_id.parse().ok().filter(|user_id| *user_id > 0)
}

#[cfg(test)]
mod tests {
    use super::parse_user_id;

    #[test]
    fn rejects_empty_user_id() {
        assert_eq!(parse_user_id(""), None);
    }

    #[test]
    fn rejects_zero_user_id() {
        assert_eq!(parse_user_id("0"), None);
        assert_eq!(parse_user_id("000"), None);
    }

    #[test]
    fn rejects_user_id_larger_than_i64() {
        assert_eq!(parse_user_id("9223372036854775808"), None);
    }

    #[test]
    fn accepts_existing_leading_zero_shape() {
        assert_eq!(parse_user_id("00042"), Some(42));
    }
}
