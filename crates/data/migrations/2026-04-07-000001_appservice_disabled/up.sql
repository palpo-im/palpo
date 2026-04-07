ALTER TABLE appservice_registrations
    ADD COLUMN disabled boolean NOT NULL DEFAULT false;
