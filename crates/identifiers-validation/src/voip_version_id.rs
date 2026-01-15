use crate::Error;
use crate::error::VoipVersionIdError;

pub fn validate(u: u64) -> Result<(), Error> {
    if u != 0 {
        return Err(VoipVersionIdError::WrongUintValue.into());
    }

    Ok(())
}
