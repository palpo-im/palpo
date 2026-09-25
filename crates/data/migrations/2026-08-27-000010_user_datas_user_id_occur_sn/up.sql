CREATE INDEX CONCURRENTLY IF NOT EXISTS user_datas_user_id_occur_sn_idx
    ON user_datas USING btree (user_id, occur_sn);
