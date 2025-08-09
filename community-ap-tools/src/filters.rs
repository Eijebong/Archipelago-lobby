use askama::Values;
use humantime::format_duration;

use crate::guards::{SlotInfo, SlotStatus};

pub fn slot_status<'a>(slot: &'a SlotInfo, _values: &'a dyn Values) -> askama::Result<&'a str> {
    if slot.status == SlotStatus::GoalCompleted {
        return Ok("green");
    }

    if slot.status != SlotStatus::Connected {
        let Some(last_activity) = slot.last_activity else {
            return Ok("red");
        };

        if last_activity > 60. * 60. {
            return Ok("red");
        }

        if last_activity > 30. * 60. {
            return Ok("yellow");
        }

        Ok("green")
    } else {
        let Some(last_activity) = slot.last_activity else {
            return Ok("yellow");
        };

        if last_activity > 60. * 60. {
            return Ok("red");
        }

        if last_activity > 30. * 60. {
            return Ok("yellow");
        }

        Ok("green")
    }
}

pub fn last_active(slot: &SlotInfo, _values: &dyn Values) -> askama::Result<String> {
    let Some(activity) = slot.last_activity else {
        return Ok("Never".to_string());
    };

    let d = std::time::Duration::from_secs(activity as u64);
    Ok(format_duration(d).to_string())
}
