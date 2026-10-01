use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MockCardToken {
    Success,
    CardDeclined,
    InsufficientFunds,
    Timeout,
    NetworkError,
}

impl FromStr for MockCardToken {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "tok_success" => Ok(Self::Success),
            "tok_card_declined" => Ok(Self::CardDeclined),
            "tok_insufficient_funds" => Ok(Self::InsufficientFunds),
            "tok_timeout" => Ok(Self::Timeout),
            "tok_network_error" => Ok(Self::NetworkError),
            _ => Err(()),
        }
    }
}
