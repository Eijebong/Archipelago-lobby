ALTER TABLE yaml_review_status DROP CONSTRAINT yaml_review_status_status_check;
ALTER TABLE yaml_review_status ADD CHECK (status IN ('unreviewed', 'reported', 'ok', 'nok'));
