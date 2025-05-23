CREATE TABLE images
(
    id         BLOB PRIMARY KEY,   -- BLAKE3 hash of the image data (256 bits)
    mime_type  TEXT      NOT NULL, -- MIME type of the image (e.g., image/png, image/jpeg)
    width      INTEGER   NOT NULL, -- Width of the image in pixels
    height     INTEGER   NOT NULL, -- Height of the image in pixels
    data       BLOB      NOT NULL, -- Image data
    created_at TIMESTAMP NOT NULL,
    updated_at TIMESTAMP NOT NULL
);

ALTER TABLE projects ADD COLUMN header_image_id BLOB;
