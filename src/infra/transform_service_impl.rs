use crate::domain::*;

use std::cell::RefCell;
use std::rc::Rc;

pub struct TransformServiceImpl {
    metadata_repo: Rc<RefCell<dyn MetadataRepository>>,
}

impl TransformServiceImpl {
    pub fn new(metadata_repo: Rc<RefCell<dyn MetadataRepository>>) -> Self {
        Self {
            metadata_repo: metadata_repo,
        }
    }

    // Note: Can be called from multiple threads
    fn do_transform(
        dataset_id: &DatasetID,
        listener: &mut dyn TransformListener,
    ) -> Result<TransformResult, TransformError> {
        let scale: f32 = rand::random();

        listener.begin();
        std::thread::sleep_ms(3000);

        let chance: f32 = rand::random();
        if rand::random::<f32>() < 0.3 {
            let error = TransformError::EngineError;
            listener.error(&error);
            Err(error)
        } else if rand::random::<f32>() < 0.3 {
            let result = TransformResult::UpToDate;
            listener.success(&result);
            Ok(result)
        } else {
            let result = TransformResult::Updated {
                block_hash: "13127948719dsdka1203ahsjkdh12983213".to_owned(),
            };
            listener.success(&result);
            Ok(result)
        }
    }
}

impl TransformService for TransformServiceImpl {
    fn transform(
        &mut self,
        dataset_id: &DatasetID,
        maybe_listener: Option<Box<dyn TransformListener>>,
    ) -> Result<TransformResult, TransformError> {
        let null_listener = Box::new(NullTransformListener {});
        let mut listener = maybe_listener.unwrap_or(null_listener);

        Self::do_transform(dataset_id, listener.as_mut())
    }

    fn transform_multi(
        &mut self,
        dataset_ids: &mut dyn Iterator<Item = &DatasetID>,
        maybe_multi_listener: Option<Box<dyn TransformMultiListener>>,
    ) -> Vec<(DatasetIDBuf, Result<TransformResult, TransformError>)> {
        let null_multi_listener = Box::new(NullTransformMultiListener {});
        let mut multi_listener = maybe_multi_listener.unwrap_or(null_multi_listener);

        let thread_handles: Vec<_> = dataset_ids
            .map(|id_ref| {
                let id = id_ref.to_owned();
                let null_listener = Box::new(NullTransformListener {});
                let mut listener = multi_listener.begin_transform(&id).unwrap_or(null_listener);
                std::thread::spawn(move || {
                    let res = Self::do_transform(&id, listener.as_mut());
                    (id, res)
                })
            })
            .collect();

        thread_handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect()
    }
}
