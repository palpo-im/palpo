-- A mapping's millisecond creation time is not an ABA-safe revision. Allocate a
-- distinct generation for every insert and update, including same-value writes.
ALTER TABLE room_aliases ADD COLUMN generation bigserial;
ALTER TABLE room_aliases ALTER COLUMN generation DROP DEFAULT;
CREATE FUNCTION advance_room_alias_generation() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    NEW.generation := nextval('room_aliases_generation_seq');
    RETURN NEW;
END;
$$;
CREATE TRIGGER advance_room_alias_generation
BEFORE INSERT OR UPDATE ON room_aliases
FOR EACH ROW EXECUTE FUNCTION advance_room_alias_generation();
