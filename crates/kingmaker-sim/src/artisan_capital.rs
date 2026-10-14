use kingmaker_core::{AccountBalance, EnterpriseRecord};
use std::collections::HashMap;

#[derive(Debug, Default)]
pub struct CapitalProtectionResult {
    pub protected_enterprises: u32,
    pub total_protected_minor: i64,
}

pub fn protect_artisan_working_capital(
    enterprises: &[EnterpriseRecord],
    accounts: &mut [AccountBalance],
) -> CapitalProtectionResult {
    let mut account_indices: HashMap<u32, usize> = HashMap::with_capacity(accounts.len());
    for (idx, acc) in accounts.iter().enumerate() {
        account_indices.insert(acc.id.0, idx);
    }

    let mut protected_enterprises = 0;
    let mut total_protected_minor = 0;

    for ent in enterprises {
        if !ent.is_artisan || ent.operating_capital.is_none_or(|value| value <= 0) {
            continue;
        }

        if let Some(&acc_idx) = ent.account_id.and_then(|id| account_indices.get(&id.0)) {
            let balance = accounts[acc_idx].balance.units();
            if balance > 0 {
                let needed = ent
                    .operating_capital
                    .expect("positive source minimum checked above");
                let protected = needed.min(balance);
                protected_enterprises += 1;
                total_protected_minor += protected;
            }
        }
    }

    CapitalProtectionResult {
        protected_enterprises,
        total_protected_minor,
    }
}
