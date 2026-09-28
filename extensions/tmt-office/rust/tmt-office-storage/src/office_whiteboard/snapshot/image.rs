//! One normalized image per retained capture, with pixel-equivalent retries.

use super::{
    Connection, DocumentError, OfficeStore, OptionalExtension, StorageError, StorageErrorCode,
    WhiteboardStoreError, classify, params, read_snapshot, valid_snapshot_id,
    with_immediate_transaction,
};
use tmt_office_model::codec::office_whiteboard::image::ValidatedSnapshotImage;

impl OfficeStore {
    pub fn attach_whiteboard_snapshot_image(
        &mut self,
        id: &str,
        image: &ValidatedSnapshotImage,
    ) -> Result<ValidatedSnapshotImage, WhiteboardStoreError> {
        if !valid_snapshot_id(id) {
            return Err(DocumentError::Invalid.into());
        }
        with_immediate_transaction(self, "whiteboard snapshot image", |transaction| {
            // Validate the retained resource, not the current mutable document.
            read_snapshot(transaction, id)?.ok_or(DocumentError::NotFound)?;
            if let Some(stored) = read_image(transaction, id)? {
                return if stored.pixel_digest() == image.pixel_digest() {
                    Ok(stored)
                } else {
                    Err(DocumentError::IdempotencyConflict.into())
                };
            }
            transaction.execute(
                "INSERT INTO office_whiteboard_snapshot_images (snapshot_id, pixel_digest, png) VALUES (?, ?, ?)",
                params![id, image.pixel_digest(), image.bytes()],
            ).map_err(|error| classify(error, "Attach whiteboard snapshot image"))?;
            Ok(image.clone())
        })
    }

    pub fn show_whiteboard_snapshot_image(
        &self,
        id: &str,
    ) -> Result<ValidatedSnapshotImage, WhiteboardStoreError> {
        if !valid_snapshot_id(id) {
            return Err(DocumentError::Invalid.into());
        }
        read_image(self.connection()?, id)?.ok_or_else(|| DocumentError::NotFound.into())
    }
}

fn read_image(
    connection: &Connection,
    id: &str,
) -> Result<Option<ValidatedSnapshotImage>, WhiteboardStoreError> {
    let row = connection
        .query_row(
            "SELECT pixel_digest, png FROM office_whiteboard_snapshot_images WHERE snapshot_id=?",
            [id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?)),
        )
        .optional()
        .map_err(|error| classify(error, "Read whiteboard snapshot image"))?;
    row.map(|(digest, bytes)| {
        ValidatedSnapshotImage::restore(bytes, &digest).map_err(|error| {
            StorageError::new(
                StorageErrorCode::Corrupt,
                "Invalid stored whiteboard snapshot image",
            )
            .caused_by(error)
            .into()
        })
    })
    .transpose()
}

#[cfg(test)]
mod tests;
