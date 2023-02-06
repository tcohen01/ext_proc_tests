use std::{sync::Arc, pin::Pin};

use futures::Stream;
use tonic::{async_trait, Code, Request as TRequest, Response as TResponse, Status, Streaming};

use crate::proto::envoy::{
    extensions::filters::http::ext_proc::v3::ProcessingMode,
    service::ext_proc::v3::{
        common_response::ResponseStatus, external_processor_server::ExternalProcessor,
        processing_request::Request, processing_response::Response, BodyResponse, CommonResponse,
        GrpcStatus, HeadersResponse, ImmediateResponse, ProcessingRequest, ProcessingResponse,
    },
};

pub struct ExtProcService {
    processing_mode: Arc<ProcessingMode>,
}

impl ExtProcService {
    pub fn new(processing_mode: ProcessingMode) -> ExtProcService {
        ExtProcService {
            processing_mode: Arc::new(processing_mode),
        }
    }
}

#[async_trait]
impl ExternalProcessor for ExtProcService {
    //type ProcessStream = ReceiverStream<Result<ProcessingResponse, Status>>;
    type ProcessStream = Pin<Box<dyn Stream<Item = Result<ProcessingResponse, Status>> + Send  + 'static>>;

    async fn process(
        &self,
        requests: TRequest<Streaming<ProcessingRequest>>,
    ) -> Result<TResponse<Self::ProcessStream>, Status> {
        let mut stream = requests.into_inner();
        let processing_mode = self.processing_mode.clone();
        let output = async_stream::try_stream! {
            while let Some(request) = stream.message().await? {
                let mut response = ExtProcService::init_response(&processing_mode);
                handle_request(request, &mut response);
                yield response;
            }
        };
        Ok(TResponse::new(Box::pin(output)))
    }
}

impl ExtProcService {
    fn init_response(processing_mode: &ProcessingMode) -> ProcessingResponse {
        ProcessingResponse{
            dynamic_metadata: None,
            mode_override: Some(processing_mode.clone()),
            response: None,
        }
    }
}

fn handle_request(request: ProcessingRequest, response: &mut ProcessingResponse) {
    match request.request {
        Some(Request::RequestHeaders(_)) => {
            response.response = Some(Response::RequestHeaders(HeadersResponse {
                response: Some(empty_response()),
            }));
        }
        Some(Request::ResponseHeaders(_)) => {
            response.response = Some(Response::ResponseHeaders(HeadersResponse {
                response: Some(empty_response()),
            }));
        }
        Some(Request::RequestBody(_)) => {
            response.response = Some(Response::RequestBody(BodyResponse {
                response: Some(empty_response()),
            }));
        }
        Some(Request::ResponseBody(_)) => {
            response.response = Some(Response::ResponseBody(BodyResponse {
                response: Some(empty_response()),
            }));
        }
        _ => {
            response.response = Some(Response::ImmediateResponse(ImmediateResponse {
                status: None,
                headers: None,
                body: String::default(),
                grpc_status: Some(GrpcStatus {
                    status: Code::InvalidArgument as u32,
                }),
                details: String::default(),
            }))
        }
    }
}

fn empty_response() -> CommonResponse {
    CommonResponse {
        status: ResponseStatus::Continue as i32,
        header_mutation: None,
        body_mutation: None,
        trailers: None,
        clear_route_cache: false,
    }
}
