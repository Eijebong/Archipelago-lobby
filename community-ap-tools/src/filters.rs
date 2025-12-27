use askama::Values;
use humantime::format_duration;

use crate::guards::{SlotInfo, SlotStatus};

#[askama::filter_fn]
pub fn slot_status(slot: &SlotInfo, _values: &dyn Values) -> askama::Result<&'static str> {
    return slot_status_fn(slot);
}

pub fn slot_status_fn(slot: &SlotInfo) -> askama::Result<&'static str> {
    if slot.status == SlotStatus::GoalCompleted {
        return Ok("green");
    }

    if ![
        SlotStatus::Connected,
        SlotStatus::Ready,
        SlotStatus::Playing,
    ]
    .contains(&slot.status)
    {
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

#[askama::filter_fn]
pub fn last_active(slot: &SlotInfo, _values: &dyn Values) -> askama::Result<String> {
    let Some(activity) = slot.last_activity else {
        return Ok("Never".to_string());
    };

    let d = std::time::Duration::from_secs(activity as u64);
    Ok(format_duration(d).to_string())
}
