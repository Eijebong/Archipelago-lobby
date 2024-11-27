-- Your SQL goes here
ALTER TABLE rooms ADD COLUMN from_template_id UUID REFERENCES room_templates(id);
