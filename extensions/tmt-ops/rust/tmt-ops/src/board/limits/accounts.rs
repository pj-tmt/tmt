//! Public account-limit acquisition and its held, typed source states.
use super::{
    Provider, Reading,
    observation::{Account, Report},
};
use crate::core::Core;
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq)]
pub(in crate::board) enum Current {
    Reported(Report),
    Waiting,
    Expired,
}
impl Current {
    pub(super) fn reading(&self) -> Option<Reading> {
        let Self::Reported(report) = self else {
            return None;
        };
        Some(Reading {
            observed_at_ms: report.observed_at_ms,
            weekly: report.window(super::QuotaWindow::Weekly)?.balance(),
            short: report
                .window(super::QuotaWindow::Short)
                .map(|point| point.balance()),
        })
    }
    pub(in crate::board) fn notice(&self) -> Option<&'static str> {
        match self {
            Self::Waiting => Some("◌ Waiting · first session since setup"),
            Self::Expired => Some("◆ Stale · no current window"),
            Self::Reported(_) => None,
        }
    }
}

pub(in crate::board) struct Accounts(pub(super) BTreeMap<Provider, Current>);
impl Accounts {
    pub(super) fn read(value: &Value) -> Option<Self> {
        value["observedAtMs"]
            .as_u64()
            .filter(|at| (1..=9_007_199_254_740_991).contains(at))?;
        let mut accounts = BTreeMap::new();
        for row in value["accounts"].as_array()? {
            let driver = row["driver"].as_str()?;
            let account = Account::read(&row["account"])?;
            let provider = Provider::from_driver(driver)?.with_account(&account);
            let current = match row.get("rateLimits")? {
                Value::Null => match row["reason"].as_str()? {
                    "none_observed" => Current::Waiting,
                    "expired" => Current::Expired,
                    _ => return None,
                },
                limits => {
                    let report = Report::read(limits)?;
                    if limits["provider"].as_str() != Some(driver)
                        || report.account.as_ref() != Some(&account)
                        || row.get("reason").is_some()
                    {
                        return None;
                    }
                    Current::Reported(report)
                }
            };
            if accounts.insert(provider, current).is_some() {
                return None;
            }
        }
        Some(Self(accounts))
    }
}

/// Compatibility belongs to the worker-owned sampler, not to rendering or a global cache.
#[derive(Default)]
pub(super) struct Reader {
    unsupported: bool,
}
impl Reader {
    pub(super) fn read(&mut self, core: &Core) -> Result<Option<Accounts>, ()> {
        if self.unsupported {
            return Ok(None);
        }
        match core.api("consumption.limits", serde_json::json!({})) {
            Ok(document) => Accounts::read(&document).map(Some).ok_or(()),
            Err(error) if error.code == "API_INPUT_INVALID" => {
                self.unsupported = true;
                Ok(None)
            }
            Err(_) => Err(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn malformed_or_mixed_accounts_never_turn_into_another_accounts_balance() {
        let account = json!({"key":"0123456789ab","label":"work"});
        let row = json!({"driver":"claude","account":account,"rateLimits":{"provider":"claude","account":account,"observedAtMs":1000,"windows":[{"windowMinutes":300,"usedPercent":8.0,"resetsAtMs":10000}]}});
        let valid = json!({"observedAtMs":2000,"accounts":[row]});
        assert!(Accounts::read(&valid).is_some());
        for path in ["provider", "account"] {
            let mut invalid = valid.clone();
            if path == "provider" {
                invalid["accounts"][0]["rateLimits"][path] = json!("codex");
            } else {
                invalid["accounts"][0]["rateLimits"][path]["key"] = json!("abcdef012345");
            }
            assert!(Accounts::read(&invalid).is_none());
        }
        let mut duplicate = valid.clone();
        duplicate["accounts"].as_array_mut().unwrap().push(row);
        assert!(Accounts::read(&duplicate).is_none());
    }
}
