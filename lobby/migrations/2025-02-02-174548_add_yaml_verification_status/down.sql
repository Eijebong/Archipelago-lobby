-- This file should undo anything in `up.sql`
ALTER TABLE yamls DROP COLUMN validation_status;
ALTER TABLE yamls DROP COLUMN apworlds;
ALTER TABLE yamls DROP COLUMN last_validation_time;
ALTER TABLE yamls DROP COLUMN last_error;
DROP TYPE VALIDATION_STATUS;
DROP TYPE APWORLD;
