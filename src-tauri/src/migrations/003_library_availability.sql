ALTER TABLE libraries
    ADD COLUMN available INTEGER NOT NULL DEFAULT 1
    CHECK(available IN (0, 1));
