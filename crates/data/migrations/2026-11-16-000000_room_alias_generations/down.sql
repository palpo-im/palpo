DROP TRIGGER advance_room_alias_generation ON room_aliases;
DROP FUNCTION advance_room_alias_generation();
-- The serial sequence is owned by this column and is dropped with it.
ALTER TABLE room_aliases DROP COLUMN generation;
