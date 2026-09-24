CREATE SEQUENCE remote_presence_recovery_seq;
ALTER TABLE remote_presence_recipients ADD COLUMN recovery_generation BIGINT NOT NULL
    DEFAULT nextval('remote_presence_recovery_seq');
