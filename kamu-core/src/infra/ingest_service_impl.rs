use super::ingest::*;
use crate::domain::*;
use crate::infra::*;

use std::cell::RefCell;
use std::rc::Rc;

pub struct IngestServiceImpl {
    workspace_layout: WorkspaceLayout,
    metadata_repo: Rc<RefCell<dyn MetadataRepository>>,
}

impl IngestServiceImpl {
    pub fn new(
        workspace_layout: &WorkspaceLayout,
        metadata_repo: Rc<RefCell<dyn MetadataRepository>>,
    ) -> Self {
        Self {
            workspace_layout: workspace_layout.clone(),
            metadata_repo: metadata_repo,
        }
    }

    // TODO: error handling
    fn get_dataset_layout(&self, dataset_id: &DatasetID) -> DatasetLayout {
        let vol = VolumeLayout::create(&self.workspace_layout.local_volume_dir).unwrap();
        DatasetLayout::create(&vol, dataset_id).unwrap()
    }
}

impl IngestService for IngestServiceImpl {
    fn ingest(
        &mut self,
        dataset_id: &DatasetID,
        maybe_listener: Option<Box<dyn IngestListener>>,
    ) -> Result<IngestResult, IngestError> {
        let null_listener = Box::new(NullIngestListener {});
        let mut listener = maybe_listener.unwrap_or(null_listener);

        let mut meta_chain = self
            .metadata_repo
            .borrow()
            .get_metadata_chain(dataset_id)
            .unwrap();

        let layout = self.get_dataset_layout(dataset_id);

        let mut ingest_task =
            IngestTask::new(dataset_id, layout, meta_chain.as_mut(), listener.as_mut());

        ingest_task.ingest()
    }

    fn ingest_multi(
        &mut self,
        dataset_ids: &mut dyn Iterator<Item = &DatasetID>,
        maybe_multi_listener: Option<&mut dyn IngestMultiListener>,
    ) -> Vec<(DatasetIDBuf, Result<IngestResult, IngestError>)> {
        let mut null_multi_listener = NullIngestMultiListener {};
        let multi_listener = maybe_multi_listener.unwrap_or(&mut null_multi_listener);

        let thread_handles: Vec<_> = dataset_ids
            .map(|id_ref| {
                let id = id_ref.to_owned();
                let layout = self.get_dataset_layout(&id);
                let mut meta_chain = self.metadata_repo.borrow().get_metadata_chain(&id).unwrap();

                let null_listener = Box::new(NullIngestListener {});
                let mut listener = multi_listener.begin_ingest(&id).unwrap_or(null_listener);

                std::thread::Builder::new()
                    .name("ingest_multi".to_owned())
                    .spawn(move || {
                        let mut ingest_task =
                            IngestTask::new(&id, layout, meta_chain.as_mut(), listener.as_mut());

                        let res = ingest_task.ingest();
                        (id, res)
                    })
                    .unwrap()
            })
            .collect();

        thread_handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect()
    }
}
