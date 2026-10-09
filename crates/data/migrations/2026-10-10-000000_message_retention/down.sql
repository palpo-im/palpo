DROP TRIGGER IF EXISTS event_retention_content_guard ON event_datas;
DROP TRIGGER IF EXISTS event_retention_search_guard ON event_searches;
DROP TRIGGER IF EXISTS event_retention_delayed_guard ON delayed_events;
DROP FUNCTION IF EXISTS palpo_retention_delayed_guard();
DROP FUNCTION IF EXISTS palpo_retention_index_guard();
DROP FUNCTION IF EXISTS palpo_retention_guard();
DROP FUNCTION IF EXISTS palpo_retention_scrub(JSON);
DROP TABLE IF EXISTS event_retention_expired;
