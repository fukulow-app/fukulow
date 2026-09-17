-- Existing channel audit records must survive a downgrade: audit history is immutable.
-- Keep the widened target_type CHECK until the identity migration drops the table.
DROP TABLE messages;
DROP TABLE channel_members;
DROP TABLE channels;
