use crate::guards::{SlotInfo, SlotStatus};

pub fn slot_status(slot: &SlotInfo) -> askama::Result<&str> {
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

        return Ok("green");
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

        return Ok("green");
    }
}
