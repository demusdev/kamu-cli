use crate::domain::*;

use std::cell::RefCell;
use std::rc::Rc;

pub struct PullServiceImpl {
    metadata_repo: Rc<RefCell<dyn MetadataRepository>>,
    ingest_svc: Rc<RefCell<dyn IngestService>>,
    transform_svc: Rc<RefCell<dyn TransformService>>,
}

impl PullServiceImpl {
    pub fn new(
        metadata_repo: Rc<RefCell<dyn MetadataRepository>>,
        ingest_svc: Rc<RefCell<dyn IngestService>>,
        transform_svc: Rc<RefCell<dyn TransformService>>,
    ) -> Self {
        Self {
            metadata_repo: metadata_repo,
            ingest_svc: ingest_svc,
            transform_svc: transform_svc,
        }
    }

    fn convert_ingest_result(
        res: Result<IngestResult, IngestError>,
    ) -> Result<PullResult, PullError> {
        match res {
            Ok(res) => Ok(match res {
                IngestResult::UpToDate => PullResult::UpToDate,
                IngestResult::Updated { block_hash } => PullResult::Updated {
                    block_hash: block_hash,
                },
            }),
            Err(err) => Err(err.into()),
        }
    }

    fn convert_transform_result(
        res: Result<TransformResult, TransformError>,
    ) -> Result<PullResult, PullError> {
        match res {
            Ok(res) => Ok(match res {
                TransformResult::UpToDate => PullResult::UpToDate,
                TransformResult::Updated { block_hash } => PullResult::Updated {
                    block_hash: block_hash,
                },
            }),
            Err(err) => Err(err.into()),
        }
    }
}

impl PullService for PullServiceImpl {
    fn pull_multi(
        &mut self,
        dataset_ids: &mut dyn Iterator<Item = &DatasetID>,
        recursive: bool,
        all: bool,
        ingest_listener: Option<Box<dyn IngestMultiListener>>,
        transform_listener: Option<Box<dyn TransformMultiListener>>,
    ) -> Vec<(DatasetIDBuf, Result<PullResult, PullError>)> {
        let metadata_repo = self.metadata_repo.borrow();

        let starting_dataset_ids: std::collections::HashSet<DatasetIDBuf> = if !all {
            dataset_ids.map(|id| id.to_owned()).collect()
        } else {
            metadata_repo.get_all_datasets().collect()
        };

        let datasets_in_order = metadata_repo.get_datasets_in_dependency_order(
            &mut starting_dataset_ids.iter().map(|id| id.as_ref()),
        );

        let datasets_to_pull = if recursive || all {
            datasets_in_order
        } else {
            datasets_in_order
                .into_iter()
                .filter(|id| starting_dataset_ids.contains(id))
                .collect()
        };

        unimplemented!();

        //let (root_datasets, deriv_datasets) = datasets_to_pull.into_iter().partition(|id| metadata_repo.)

        /*results.extend(
            self.ingest_svc
                .borrow_mut()
                .ingest_multi(
                    &mut dataset_ids.iter().map(|id| id.as_ref()),
                    ingest_listener,
                )
                .into_iter()
                .map(|(id, res)| (id, Self::convert_ingest_result(res))),
        );

        results.extend(
            self.transform_svc
                .borrow_mut()
                .transform_multi(
                    &mut dataset_ids.iter().map(|id| id.as_ref()),
                    transform_listener,
                )
                .into_iter()
                .map(|(id, res)| (id, Self::convert_transform_result(res))),
        );*/
    }
}
