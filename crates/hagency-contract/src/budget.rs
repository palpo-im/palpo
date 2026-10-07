//! Budget observations are not reservations. The writer must lock/read the
//! parent and engagement records and persist any reservation in one transaction.
use serde::{Deserialize, Serialize};

use crate::{Error, Tokens};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BudgetSnapshot {
    pub allocated: Option<Tokens>,
    pub consumed: Tokens,
    pub reserved_unused: Tokens,
}

impl BudgetSnapshot {
    /// Consumed capacity never becomes available by retiring an agent. An
    /// overdrawn observation returns an error rather than pretending balance 0
    /// is evidence that no excess usage occurred.
    pub fn available(&self) -> Result<Tokens, Error> {
        let allocated = u64::from(self.allocated.ok_or(Error::Unallocated)?);
        let charged = u64::from(self.consumed)
            .checked_add(u64::from(self.reserved_unused))
            .ok_or(Error::Overflow)?;
        Tokens::try_from(
            allocated
                .checked_sub(charged)
                .ok_or(Error::InsufficientCapacity)?,
        )
    }

    /// Use for a new reservation or a top-up delta, never the agent's new total.
    /// The parent grant has already reserved engagement capacity; do not debit
    /// it a second time for the same child reservation.
    pub fn check_increase(&self, additional: Tokens) -> Result<(), Error> {
        if u64::from(additional) == 0 {
            return Err(Error::InvalidNumber);
        }
        if additional > self.available()? {
            return Err(Error::InsufficientCapacity);
        }
        Ok(())
    }
}
