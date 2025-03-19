CREATE TABLE frogress_mappings
(
    frogress_slug     TEXT    NOT NULL,
    frogress_version  TEXT    NOT NULL,
    frogress_category TEXT    NOT NULL,
    frogress_measure  TEXT    NOT NULL,
    project_id        INTEGER NOT NULL,
    version           TEXT    NOT NULL,
    category          TEXT    NOT NULL,
    category_name     TEXT    NOT NULL,
    measure           TEXT    NOT NULL,
    PRIMARY KEY (frogress_slug, frogress_version, frogress_category, frogress_measure),
    FOREIGN KEY (project_id) REFERENCES projects (id)
);
