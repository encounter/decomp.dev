use axum::{
    http::header,
    response::{IntoResponse, Response},
};
use bytes::BytesMut;
use prost::Message;

pub struct Protobuf<'a, T>(pub &'a T)
where T: Message;

pub const APPLICATION_PROTOBUF: &str = "application/x-protobuf";
pub const PROTOBUF: &str = "x-protobuf";

impl<T: Message> IntoResponse for Protobuf<'_, T> {
    fn into_response(self) -> Response {
        let mut bytes = BytesMut::with_capacity(self.0.encoded_len());
        self.0.encode(&mut bytes).unwrap();
        ([(header::CONTENT_TYPE, APPLICATION_PROTOBUF)], bytes.freeze()).into_response()
    }
}
