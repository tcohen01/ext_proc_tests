use std::sync::Arc;

use tokio::sync::mpsc::Sender;
use tonic::{transport::Channel, Status, Streaming};

use crate::proto::envoy::{
    config::core::v3::{HeaderMap, HeaderValue},
    extensions::filters::http::ext_proc::v3::{
        processing_mode::{BodySendMode, HeaderSendMode},
        ProcessingMode,
    },
    service::ext_proc::v3::{
        external_processor_client::ExternalProcessorClient, processing_request::Request, HttpBody,
        HttpHeaders, ProcessingRequest, ProcessingResponse,
    },
};

use super::DummyData;
use error::StreamHandleError;

pub mod error {
    use crate::proto::envoy::service::ext_proc::v3::ProcessingRequest;
    use quick_error::quick_error;
    use tokio::sync::mpsc::error::SendError;
    use tonic::Status;
    quick_error!(
        #[derive(Debug)]
        pub enum StreamHandleError {
            RequestSendError(err: Box<SendError<ProcessingRequest>>) {
                from(err: SendError<ProcessingRequest>) -> (Box::new(err))
            }
            ResponseError(err: Status) {
                from()
            }
            StreamClosed {
                display("Stream closed unexpectedly.")
            }
        }
    );
}

/// A stream handler for ExternalProcessorClient that sends dummy data to the service.
/// In a real world scenario this would be an RPC Stream Handler that can be pooled.
pub struct ClientStream {
    data: Arc<DummyData>,
    config: Config,

    request_sender: Option<Sender<ProcessingRequest>>,
    response_receiver: Option<Streaming<ProcessingResponse>>,

    state: StreamState,
}

struct StreamState {
    processing_mode: ProcessingMode,
    handle_count: usize,
}

impl Default for StreamState {
    fn default() -> Self {
        Self {
            processing_mode: ProcessingMode {
                request_header_mode: HeaderSendMode::Send.into(),
                response_header_mode: HeaderSendMode::Send.into(),
                request_body_mode: BodySendMode::Buffered.into(),
                response_body_mode: BodySendMode::Buffered.into(),
                request_trailer_mode: HeaderSendMode::Skip.into(),
                response_trailer_mode: HeaderSendMode::Skip.into(),
            },
            handle_count: 0,
        }
    }
}

/// Reuse stream configuration
#[derive(Clone)]
pub struct Config {
    /// Prevent stream from being closed on calls to finish_stream (streams may still be closed if max_handled is reached)
    pub reuse_stream: bool,
    /// Implemented as hardcap, but this can also be implemented as a softcap
    /// (chance to close stream using fastrnd until a hardcap, to prevent stream creation spikes)
    pub max_handled: Option<usize>,
}

impl StreamState {
    fn set_request_header_mode(&mut self, mode: HeaderSendMode) {
        if let HeaderSendMode::Default = mode as HeaderSendMode {
            self.processing_mode
                .set_request_header_mode(HeaderSendMode::Send);
        } else {
            self.processing_mode.set_request_header_mode(mode);
        }
    }
    fn set_response_header_mode(&mut self, mode: HeaderSendMode) {
        if let HeaderSendMode::Default = mode as HeaderSendMode {
            self.processing_mode
                .set_response_header_mode(HeaderSendMode::Send);
        } else {
            self.processing_mode.set_response_header_mode(mode);
        }
    }
    fn set_request_trailer_mode(&mut self, mode: HeaderSendMode) {
        if let HeaderSendMode::Default = mode as HeaderSendMode {
            self.processing_mode
                .set_request_trailer_mode(HeaderSendMode::Skip);
        } else {
            self.processing_mode.set_request_trailer_mode(mode);
        }
    }
    fn set_response_trailer_mode(&mut self, mode: HeaderSendMode) {
        if let HeaderSendMode::Default = mode as HeaderSendMode {
            self.processing_mode
                .set_response_trailer_mode(HeaderSendMode::Skip);
        } else {
            self.processing_mode.set_response_trailer_mode(mode);
        }
    }
}

trait StreamHandleRef<T> {
    fn as_expected_ref(&self) -> &T;
}

impl StreamHandleRef<Sender<ProcessingRequest>> for Option<Sender<ProcessingRequest>> {
    fn as_expected_ref(&self) -> &Sender<ProcessingRequest> {
        self.as_ref()
            .expect("Must be used after start_stream but before finish_stream")
    }
}

trait StreamHandleMutRef<T> {
    fn as_expected_mut(&mut self) -> &mut T;
}

impl StreamHandleMutRef<Streaming<ProcessingResponse>> for Option<Streaming<ProcessingResponse>> {
    fn as_expected_mut(&mut self) -> &mut Streaming<ProcessingResponse> {
        self.as_mut()
            .expect("Must be used after start_stream but before finish_stream")
    }
}

impl ClientStream {
    pub fn new(data: Arc<DummyData>, config: Config) -> ClientStream {
        ClientStream {
            data,
            config,
            request_sender: None,
            response_receiver: None,
            state: Default::default(),
        }
    }

    pub async fn start_stream(
        &mut self,
        client: &mut ExternalProcessorClient<Channel>,
    ) -> Result<(), Status> {
        if matches!(self.request_sender, Some(ref sender) if !sender.is_closed())
            && self.response_receiver.is_some()
        {
            return Ok(());
        }

        let (tx, rx) = tokio::sync::mpsc::channel(4);
        let response = client
            .process(tokio_stream::wrappers::ReceiverStream::new(rx))
            .await?;

        self.request_sender = Some(tx);
        self.response_receiver = Some(response.into_inner());
        self.state = Default::default();
        Ok(())
    }

    pub fn sender_as_ref(
        request_sender: &Option<Sender<ProcessingRequest>>,
    ) -> &Sender<ProcessingRequest> {
        request_sender
            .as_ref()
            .expect("Must be used after start_stream but before finish_stream")
    }

    pub fn receiver_as_ref(
        response_receiver: &mut Option<Streaming<ProcessingResponse>>,
    ) -> &mut Streaming<ProcessingResponse> {
        response_receiver
            .as_mut()
            .expect("Must be used after start_stream but before finish_stream")
    }

    pub async fn handle_stream(&mut self) -> Result<(), StreamHandleError> {
        async fn send_headers<F: FnOnce(HttpHeaders) -> Request>(
            into_request: F,
            sender: &Sender<ProcessingRequest>,
            headers: &[(String, String)],
            end_of_stream: bool,
        ) -> Result<(), StreamHandleError> {
            let headers_map = HeaderMap {
                headers: headers
                    .iter()
                    .map(|header| HeaderValue {
                        key: header.0.to_lowercase(),
                        value: header.1.clone(),
                    })
                    .collect(),
            };
            Ok(sender
                .send(ProcessingRequest {
                    async_mode: false,
                    request: Some(into_request(HttpHeaders {
                        headers: Some(headers_map),
                        attributes: Default::default(),
                        end_of_stream,
                    })),
                })
                .await?)
        }
        async fn send_body<F: FnOnce(HttpBody) -> Request>(
            into_request: F,
            sender: &Sender<ProcessingRequest>,
            body: &[u8],
            end_of_stream: bool,
        ) -> Result<(), StreamHandleError> {
            Ok(sender
                .send(ProcessingRequest {
                    async_mode: false,
                    request: Some(into_request(HttpBody {
                        body: Vec::from(body),
                        end_of_stream,
                    })),
                })
                .await?)
        }
        async fn send_empty(_sender: &Sender<ProcessingRequest>) -> Result<(), StreamHandleError> {
            Ok(())
        }

        /*let response_receiver = self.response_receiver
        .as_mut()
        .expect("Must be used after start_stream but before finish_stream");*/

        if self.state.processing_mode.request_header_mode() != HeaderSendMode::Skip {
            send_headers(
                |headers| Request::RequestHeaders(headers),
                self.request_sender.as_expected_ref(),
                &self.data.req_headers,
                self.data.req_body.is_empty(),
            )
            .await?;
            self.process_single_response().await?;
        }
        if self.state.processing_mode.request_body_mode() != BodySendMode::None
            && !self.data.req_body.is_empty()
        {
            send_body(
                |body| Request::RequestBody(body),
                self.request_sender.as_expected_ref(),
                &self.data.req_body,
                true,
            )
            .await?;
            self.process_single_response().await?;
        }
        if self.state.processing_mode.response_header_mode() != HeaderSendMode::Skip {
            send_headers(
                |headers| Request::ResponseHeaders(headers),
                self.request_sender.as_expected_ref(),
                &self.data.resp_headers,
                self.data.resp_body.is_empty(),
            )
            .await?;
            self.process_single_response().await?;
        }
        if self.state.processing_mode.response_body_mode() != BodySendMode::None
            && !self.data.resp_body.is_empty()
        {
            send_body(
                |body| Request::ResponseBody(body),
                self.request_sender.as_expected_ref(),
                &self.data.req_body,
                true,
            )
            .await?;
            self.process_single_response().await?;
        }
        self.state.handle_count += 1;
        Ok(())
    }

    pub async fn process_single_response(&mut self) -> Result<(), StreamHandleError> {
        let response = self.response_receiver.as_expected_mut().message().await?;
        if let None = response {
            return Err(StreamHandleError::StreamClosed);
        }

        let response = response.unwrap();
        if let Some(mode_overrides) = response.mode_override {
            self.state
                .set_request_header_mode(mode_overrides.request_header_mode());
            self.state
                .set_response_header_mode(mode_overrides.response_header_mode());
            self.state
                .processing_mode
                .set_request_body_mode(mode_overrides.request_body_mode());
            self.state
                .processing_mode
                .set_response_body_mode(mode_overrides.response_body_mode());
            self.state
                .set_request_trailer_mode(mode_overrides.request_trailer_mode());
            self.state
                .set_response_trailer_mode(mode_overrides.response_trailer_mode());
        }

        Ok(())
    }

    pub fn finish_stream(&mut self) {
        if !self.config.reuse_stream
            || matches!(self.config.max_handled, Some(ref max) if self.state.handle_count >= *max)
        {
            self.request_sender = None;
            self.response_receiver = None;
        }
    }
}
