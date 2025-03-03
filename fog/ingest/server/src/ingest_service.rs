// Copyright (c) 2018-2021 The MobileCoin Foundation

//! Implement the ingest grpc API

use crate::{controller::IngestController, error::IngestServiceError};
use fog_api::{
    fog_common::BlockRange,
    ingest::*,
    ingest_common::{IngestSummary, SetPeersRequest},
    Empty,
};
use fog_recovery_db_iface::{RecoveryDb, ReportDb};
use fog_uri::IngestPeerUri;
use grpcio::{RpcContext, RpcStatus, UnarySink};
use mc_attest_net::RaClient;
use mc_common::logger::Logger;
use mc_ledger_db::{Ledger, LedgerDB};
use mc_util_grpc::{
    rpc_database_err, rpc_invalid_arg_error, rpc_logger, rpc_precondition_error, send_result,
};
use mc_util_metrics::SVC_COUNTERS;
use protobuf::RepeatedField;
use std::{str::FromStr, sync::Arc};

/// Implements the ingest grpc api
#[derive(Clone)]
pub struct IngestService<
    R: RaClient + Send + Sync + 'static,
    DB: RecoveryDb + ReportDb + Clone + Send + Sync + 'static,
> where
    IngestServiceError: From<<DB as RecoveryDb>::Error>,
{
    controller: Arc<IngestController<R, DB>>,
    ledger_db: LedgerDB,
    logger: Logger,
}

impl<
        R: RaClient + Send + Sync + 'static,
        DB: RecoveryDb + ReportDb + Clone + Send + Sync + 'static,
    > IngestService<R, DB>
where
    IngestServiceError: From<<DB as RecoveryDb>::Error>,
{
    /// Creates a new ingest node (but does not create sockets and start it etc.)
    pub fn new(
        controller: Arc<IngestController<R, DB>>,
        ledger_db: LedgerDB,
        logger: Logger,
    ) -> Self {
        Self {
            controller,
            ledger_db,
            logger,
        }
    }

    /// Logic of proto api
    pub fn get_status_impl(&mut self) -> Result<IngestSummary, RpcStatus> {
        Ok(self.controller.get_ingest_summary())
    }

    /// Logic of proto api
    pub fn new_keys_impl(&mut self, logger: &Logger) -> Result<IngestSummary, RpcStatus> {
        self.controller
            .new_keys()
            .map_err(|err| rpc_precondition_error("new_keys", err, logger))?;

        Ok(self.controller.get_ingest_summary())
    }

    /// Logic of proto api
    pub fn set_pubkey_expiry_window_impl(
        &mut self,
        request: SetPubkeyExpiryWindowRequest,
        logger: &Logger,
    ) -> Result<IngestSummary, RpcStatus> {
        self.controller
            .set_pubkey_expiry_window(request.pubkey_expiry_window)
            .map_err(|err| rpc_precondition_error("set_pubkey_expiry_window", err, logger))?;

        Ok(self.controller.get_ingest_summary())
    }

    /// Logic of proto api
    pub fn set_peers_impl(
        &mut self,
        request: SetPeersRequest,
        logger: &Logger,
    ) -> Result<IngestSummary, RpcStatus> {
        let peers = request
            .ingest_peer_uris
            .iter()
            .map(|x| IngestPeerUri::from_str(x))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| rpc_invalid_arg_error("invalid peer uri", err, logger))?;

        self.controller.set_peers(peers);

        Ok(self.controller.get_ingest_summary())
    }

    /// Logic of proto api
    pub fn activate_impl(&mut self, _: Empty, logger: &Logger) -> Result<IngestSummary, RpcStatus> {
        self.controller
            .activate(
                self.ledger_db
                    .num_blocks()
                    .map_err(|err| rpc_database_err(err, logger))?,
            )
            .map_err(|err| rpc_precondition_error("activate", err, logger))?;

        Ok(self.controller.get_ingest_summary())
    }

    /// Logic of proto api
    pub fn retire_impl(&mut self, _: Empty, logger: &Logger) -> Result<IngestSummary, RpcStatus> {
        Ok(self
            .controller
            .retire()
            .map_err(|err| rpc_database_err(err, logger))?)
    }

    /// Report a missed block range
    pub fn report_missed_block_range_impl(
        &mut self,
        request: ReportMissedBlockRangeRequest,
        logger: &Logger,
    ) -> Result<Empty, RpcStatus> {
        let block_range =
            fog_types::common::BlockRange::new(request.start_index, request.end_index);
        if !block_range.is_valid() {
            return Err(rpc_invalid_arg_error(
                "block_range",
                "invalid range",
                logger,
            ));
        }

        self.controller
            .report_missed_block_range(&block_range)
            .map_err(|err| rpc_database_err(err, logger))?;

        Ok(Empty::new())
    }

    /// Gets all the known missed block ranges
    pub fn get_missed_block_ranges_impl(
        &mut self,
        logger: &Logger,
    ) -> Result<GetMissedBlockRangesResponse, RpcStatus> {
        let ranges = self
            .controller
            .get_missed_block_ranges()
            .map_err(|err| rpc_database_err(err, logger))?;

        let mut response = GetMissedBlockRangesResponse::new();
        response.set_missed_block_ranges(RepeatedField::from_vec(
            ranges
                .iter()
                .map(|range| {
                    let mut proto_range = BlockRange::new();
                    proto_range.set_start_block(range.start_block);
                    proto_range.set_end_block(range.end_block);
                    proto_range
                })
                .collect(),
        ));

        Ok(response)
    }
}

impl<
        R: RaClient + Send + Sync + 'static,
        DB: RecoveryDb + ReportDb + Clone + Send + Sync + 'static,
    > fog_api::ingest_grpc::AccountIngestApi for IngestService<R, DB>
where
    IngestServiceError: From<<DB as RecoveryDb>::Error>,
{
    fn get_status(&mut self, ctx: RpcContext, _request: Empty, sink: UnarySink<IngestSummary>) {
        let _timer = SVC_COUNTERS.req(&ctx);
        mc_common::logger::scoped_global_logger(&rpc_logger(&ctx, &self.logger), |logger| {
            send_result(ctx, sink, self.get_status_impl(), &logger)
        })
    }

    fn new_keys(&mut self, ctx: RpcContext, _request: Empty, sink: UnarySink<IngestSummary>) {
        let _timer = SVC_COUNTERS.req(&ctx);
        mc_common::logger::scoped_global_logger(&rpc_logger(&ctx, &self.logger), |logger| {
            send_result(ctx, sink, self.new_keys_impl(&logger), &logger)
        })
    }

    fn set_pubkey_expiry_window(
        &mut self,
        ctx: RpcContext,
        request: SetPubkeyExpiryWindowRequest,
        sink: UnarySink<IngestSummary>,
    ) {
        let _timer = SVC_COUNTERS.req(&ctx);
        mc_common::logger::scoped_global_logger(&rpc_logger(&ctx, &self.logger), |logger| {
            send_result(
                ctx,
                sink,
                self.set_pubkey_expiry_window_impl(request, &logger),
                &logger,
            )
        })
    }

    fn set_peers(
        &mut self,
        ctx: RpcContext,
        request: SetPeersRequest,
        sink: UnarySink<IngestSummary>,
    ) {
        let _timer = SVC_COUNTERS.req(&ctx);
        mc_common::logger::scoped_global_logger(&rpc_logger(&ctx, &self.logger), |logger| {
            send_result(ctx, sink, self.set_peers_impl(request, &logger), &logger)
        })
    }

    fn activate(&mut self, ctx: RpcContext, request: Empty, sink: UnarySink<IngestSummary>) {
        let _timer = SVC_COUNTERS.req(&ctx);
        mc_common::logger::scoped_global_logger(&rpc_logger(&ctx, &self.logger), |logger| {
            send_result(ctx, sink, self.activate_impl(request, &logger), &logger)
        })
    }

    fn retire(&mut self, ctx: RpcContext, request: Empty, sink: UnarySink<IngestSummary>) {
        let _timer = SVC_COUNTERS.req(&ctx);
        mc_common::logger::scoped_global_logger(&rpc_logger(&ctx, &self.logger), |logger| {
            send_result(ctx, sink, self.retire_impl(request, &logger), &logger)
        })
    }

    fn report_missed_block_range(
        &mut self,
        ctx: RpcContext,
        request: ReportMissedBlockRangeRequest,
        sink: UnarySink<Empty>,
    ) {
        let _timer = SVC_COUNTERS.req(&ctx);
        mc_common::logger::scoped_global_logger(&rpc_logger(&ctx, &self.logger), |logger| {
            send_result(
                ctx,
                sink,
                self.report_missed_block_range_impl(request, &logger),
                &logger,
            )
        })
    }

    fn get_missed_block_ranges(
        &mut self,
        ctx: RpcContext,
        _request: Empty,
        sink: UnarySink<GetMissedBlockRangesResponse>,
    ) {
        let _timer = SVC_COUNTERS.req(&ctx);
        mc_common::logger::scoped_global_logger(&rpc_logger(&ctx, &self.logger), |logger| {
            send_result(
                ctx,
                sink,
                self.get_missed_block_ranges_impl(&logger),
                &logger,
            )
        })
    }
}
