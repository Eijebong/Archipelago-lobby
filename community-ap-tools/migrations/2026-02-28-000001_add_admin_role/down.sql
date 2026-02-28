UPDATE team_members SET role = 'editor' WHERE role = 'admin';
ALTER TABLE team_members DROP CONSTRAINT team_members_role_check;
ALTER TABLE team_members ADD CONSTRAINT team_members_role_check
  CHECK (role IN ('viewer', 'reviewer', 'rule_editor', 'editor'));
