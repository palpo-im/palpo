//! Account-owned preferences, compatible with the legacy Inbox representation.
//! A preference changes delivery, never the action or its business authority.
use crate::{Result, digest, fail, workflow::Workflows};
use jiff::{Timestamp, tz::TimeZone};
use palpo_hagency_contract::MatrixUserId;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct QuietHours {
    start: String,
    end: String,
    time_zone: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Preferences {
    pub enabled: bool,
    pub reminders_enabled: bool,
    pub reminder_minutes: Vec<u64>,
    quiet_hours: Option<QuietHours>,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            enabled: true,
            reminders_enabled: true,
            reminder_minutes: vec![60, 1440, 2880],
            quiet_hours: None,
        }
    }
}
fn minutes(s: &str) -> Result<u16> {
    let b = s.as_bytes();
    if b.len() != 5 || b[2] != b':' || ![b[0], b[1], b[3], b[4]].iter().all(u8::is_ascii_digit) {
        return Err(fail(400, "invalid_notification_preferences"));
    }
    let hour = (b[0] - b'0') as u16 * 10 + (b[1] - b'0') as u16;
    let minute = (b[3] - b'0') as u16 * 10 + (b[4] - b'0') as u16;
    if hour > 23 || minute > 59 {
        return Err(fail(400, "invalid_notification_preferences"));
    }
    Ok(hour * 60 + minute)
}
impl Preferences {
    fn validate(&self) -> Result<()> {
        if self.reminder_minutes.is_empty()
            || self.reminder_minutes.len() > 3
            || self
                .reminder_minutes
                .iter()
                .any(|v| !(15..=10080).contains(v))
            || self.reminder_minutes.windows(2).any(|v| v[0] >= v[1])
        {
            return Err(fail(400, "invalid_notification_preferences"));
        }
        if let Some(q) = &self.quiet_hours
            && (minutes(&q.start)? == minutes(&q.end)?
                || q.time_zone.len() > 100
                || TimeZone::get(&q.time_zone).is_err())
        {
            return Err(fail(400, "invalid_notification_preferences"));
        }
        Ok(())
    }
    pub fn quiet(&self, now: u64) -> Result<bool> {
        let Some(q) = &self.quiet_hours else {
            return Ok(false);
        };
        let zone = TimeZone::get(&q.time_zone)
            .map_err(|_| fail(503, "notification_time_zone_unavailable"))?;
        let time = Timestamp::from_millisecond(
            i64::try_from(now).map_err(|_| fail(400, "invalid_clock"))?,
        )
        .map_err(|_| fail(400, "invalid_clock"))?
        .to_zoned(zone);
        let at = time.hour() as u16 * 60 + time.minute() as u16;
        let start = minutes(&q.start)?;
        let end = minutes(&q.end)?;
        Ok(if start < end {
            at >= start && at < end
        } else {
            at >= start || at < end
        })
    }
}
pub(crate) fn load(state: &Value, actor: &MatrixUserId) -> Result<Preferences> {
    let value = &state["actionInbox"]["preferences"][actor.as_str()]["value"];
    let prefs: Preferences = if value.is_null() {
        Preferences::default()
    } else {
        serde_json::from_value(value.clone())?
    };
    prefs.validate()?;
    Ok(prefs)
}
pub(crate) fn get(state: &Value, actor: &MatrixUserId) -> Result<Value> {
    let mut value = serde_json::to_value(load(state, actor)?)?;
    value["revision"] = json!(
        state["actionInbox"]["preferences"][actor.as_str()]["revision"]
            .as_u64()
            .unwrap_or(0)
    );
    Ok(value)
}
pub(crate) fn set(
    state: &mut Value,
    w: &mut Workflows,
    mut input: Value,
    actor: &MatrixUserId,
    now: u64,
) -> Result<Value> {
    let expected = input
        .as_object_mut()
        .and_then(|o| o.remove("expectedRevision"))
        .and_then(|v| v.as_u64())
        .filter(|n| *n < 9_007_199_254_740_991)
        .ok_or_else(|| fail(400, "invalid_notification_preferences"))?;
    // Text inputs in the existing Splash contract send decimal strings.
    if let Some(values) = input["reminderMinutes"].as_array_mut() {
        for v in values {
            if let Some(s) = v.as_str() {
                if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(fail(400, "invalid_notification_preferences"));
                }
                *v = json!(
                    s.parse::<u64>()
                        .map_err(|_| fail(400, "invalid_notification_preferences"))?
                );
            }
        }
    }
    let prefs: Preferences =
        serde_json::from_value(input).map_err(|_| fail(400, "invalid_notification_preferences"))?;
    prefs.validate()?;
    let value = serde_json::to_value(&prefs)?;
    let fingerprint = digest(&json!({"expectedRevision":expected,"value":value}))?;
    let previous = &state["actionInbox"]["preferences"][actor.as_str()];
    if previous["fingerprint"] == fingerprint {
        return get(state, actor);
    }
    if previous["revision"].as_u64().unwrap_or(0) != expected {
        return Err(fail(409, "notification_preferences_changed"));
    }
    if previous.is_null()
        && state["actionInbox"]["preferences"]
            .as_object()
            .is_some_and(|p| p.len() >= 10000)
    {
        return Err(fail(429, "notification_preferences_full"));
    }
    let prior = load(state, actor)?;
    let cadence_changed = prior.reminder_minutes != prefs.reminder_minutes;
    if !state["actionInbox"].is_object() {
        state["actionInbox"] = json!({});
    }
    if !state["actionInbox"]["preferences"].is_object() {
        state["actionInbox"]["preferences"] = json!({});
    }
    state["actionInbox"]["preferences"][actor.as_str()] =
        json!({"revision":expected+1,"fingerprint":fingerprint,"value":value});
    for n in w
        .notices
        .values_mut()
        .filter(|n| n["recipient"] == actor.as_str() && n["cancelled"] != true)
    {
        let delivered = n["delivered"].as_u64().unwrap_or(0);
        if cadence_changed {
            n["reminderCursor"] = json!(delivered.saturating_sub(1));
            n["finished"] = json!(delivered > prefs.reminder_minutes.len() as u64);
        }
        if cadence_changed
            || !prior.enabled && prefs.enabled
            || !prior.reminders_enabled && prefs.reminders_enabled
        {
            let delay = if delivered == 0 {
                0
            } else {
                prefs
                    .reminder_minutes
                    .get(n["reminderCursor"].as_u64().unwrap_or(delivered - 1) as usize)
                    .copied()
                    .unwrap_or(0)
                    * 60000
            };
            n["dueAt"] = json!(
                n["createdAt"]
                    .as_u64()
                    .unwrap_or(now)
                    .saturating_add(delay)
                    .max(n["snoozedUntil"].as_u64().unwrap_or(0))
            );
        }
    }
    state["audit"]
        .as_array_mut()
        .ok_or_else(|| fail(503, "workflow_state_invalid"))?
        .push(
            json!({"atMs":now,"actor":actor,"action":"notifications.preferences","result":"saved"}),
        );
    get(state, actor)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn at(s: &str) -> u64 {
        s.parse::<Timestamp>().unwrap().as_millisecond() as u64
    }
    #[test]
    fn quiet_hours_follow_both_folds_and_the_spring_gap() {
        let p: Preferences = serde_json::from_value(
            json!({"enabled":true,"remindersEnabled":true,"reminderMinutes":[60],
            "quietHours":{"start":"01:00","end":"03:00","timeZone":"America/Los_Angeles"}}),
        )
        .unwrap();
        p.validate().unwrap();
        assert!(p.quiet(at("2026-11-01T08:30:00Z")).unwrap());
        assert!(p.quiet(at("2026-11-01T09:30:00Z")).unwrap());
        assert!(!p.quiet(at("2026-11-01T11:00:00Z")).unwrap());
        assert!(p.quiet(at("2026-03-08T09:59:00Z")).unwrap());
        assert!(!p.quiet(at("2026-03-08T10:00:00Z")).unwrap());
    }
}
