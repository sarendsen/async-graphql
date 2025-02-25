use std::any::Any;
use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::fmt::{self, Debug, Display, Formatter};
use std::marker::PhantomData;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{parser, InputType, Pos, Value};

/// Extensions to the error.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(transparent)]
pub struct ErrorExtensionValues(BTreeMap<String, Value>);

impl ErrorExtensionValues {
    /// Set an extension value.
    pub fn set(&mut self, name: impl AsRef<str>, value: impl Into<Value>) {
        self.0.insert(name.as_ref().to_string(), value.into());
    }
}

/// An error in a GraphQL server.
#[derive(Clone, Serialize, Deserialize)]
pub struct ServerError {
    /// An explanatory message of the error.
    pub message: String,
    /// The source of the error, comes from [`ResolverError<T>`].
    #[serde(skip)]
    pub source: Option<Arc<dyn Any + Send + Sync>>,
    /// Where the error occurred.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub locations: Vec<Pos>,
    /// If the error occurred in a resolver, the path to the error.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub path: Vec<PathSegment>,
    /// Extensions to the error.
    #[serde(skip_serializing_if = "error_extensions_is_empty", default)]
    pub extensions: Option<ErrorExtensionValues>,
}

fn error_extensions_is_empty(values: &Option<ErrorExtensionValues>) -> bool {
    values.as_ref().map_or(true, |values| values.0.is_empty())
}

impl Debug for ServerError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("ServerError")
            .field("message", &self.message)
            .field("locations", &self.locations)
            .field("path", &self.path)
            .field("extensions", &self.extensions)
            .finish()
    }
}

impl PartialEq for ServerError {
    fn eq(&self, other: &Self) -> bool {
        self.message.eq(&other.message)
            && self.locations.eq(&other.locations)
            && self.path.eq(&other.path)
            && self.extensions.eq(&other.extensions)
    }
}

impl ServerError {
    /// Create a new server error with the message.
    pub fn new(message: impl Into<String>, pos: Option<Pos>) -> Self {
        Self {
            message: message.into(),
            source: None,
            locations: pos.map(|pos| vec![pos]).unwrap_or_default(),
            path: Vec::new(),
            extensions: None,
        }
    }

    /// Get the source of the error.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use std::string::FromUtf8Error;
    /// use async_graphql::{ResolverError, Error, ServerError, Pos};
    ///
    /// let bytes = vec![0, 159];
    /// let err: Error = String::from_utf8(bytes).map_err(ResolverError::new).unwrap_err().into();
    /// let server_err: ServerError = err.into_server_error(Pos { line: 1, column: 1 });
    /// assert!(server_err.source::<FromUtf8Error>().is_some());
    /// ```
    pub fn source<T: Any + Send + Sync>(&self) -> Option<&T> {
        self.source.as_ref().map(|err| err.downcast_ref()).flatten()
    }

    #[doc(hidden)]
    pub fn with_path(self, path: Vec<PathSegment>) -> Self {
        Self { path, ..self }
    }
}

impl Display for ServerError {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl From<ServerError> for Vec<ServerError> {
    fn from(single: ServerError) -> Self {
        vec![single]
    }
}

impl From<parser::Error> for ServerError {
    fn from(e: parser::Error) -> Self {
        Self {
            message: e.to_string(),
            source: None,
            locations: e.positions().collect(),
            path: Vec::new(),
            extensions: None,
        }
    }
}

/// A segment of path to a resolver.
///
/// This is like [`QueryPathSegment`](enum.QueryPathSegment.html), but owned and used as a part of
/// errors instead of during execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PathSegment {
    /// A field in an object.
    Field(String),
    /// An index in a list.
    Index(usize),
}

/// Alias for `Result<T, ServerError>`.
pub type ServerResult<T> = std::result::Result<T, ServerError>;

/// An error parsing an input value.
///
/// This type is generic over T as it uses T's type name when converting to a regular error.
#[derive(Debug)]
pub struct InputValueError<T> {
    message: String,
    phantom: PhantomData<T>,
}

impl<T: InputType> InputValueError<T> {
    fn new(message: String) -> Self {
        Self {
            message,
            phantom: PhantomData,
        }
    }

    /// The expected input type did not match the actual input type.
    #[must_use]
    pub fn expected_type(actual: Value) -> Self {
        Self::new(format!(
            r#"Expected input type "{}", found {}."#,
            T::type_name(),
            actual
        ))
    }

    /// A custom error message.
    ///
    /// Any type that implements `Display` is automatically converted to this if you use the `?`
    /// operator.
    #[must_use]
    pub fn custom(msg: impl Display) -> Self {
        Self::new(format!(r#"Failed to parse "{}": {}"#, T::type_name(), msg))
    }

    /// Propagate the error message to a different type.
    pub fn propagate<U: InputType>(self) -> InputValueError<U> {
        if T::type_name() != U::type_name() {
            InputValueError::new(format!(
                r#"{} (occurred while parsing "{}")"#,
                self.message,
                U::type_name()
            ))
        } else {
            InputValueError::new(self.message)
        }
    }

    /// Convert the error into a server error.
    pub fn into_server_error(self, pos: Pos) -> ServerError {
        ServerError::new(self.message, Some(pos))
    }
}

impl<T: InputType, E: Display> From<E> for InputValueError<T> {
    fn from(error: E) -> Self {
        Self::custom(error)
    }
}

/// An error parsing a value of type `T`.
pub type InputValueResult<T> = Result<T, InputValueError<T>>;

/// An error with a message and optional extensions.
#[derive(Clone, Serialize)]
pub struct Error {
    /// The error message.
    pub message: String,
    /// The source of the error, comes from [`ResolverError<T>`].
    #[serde(skip)]
    pub source: Option<Arc<dyn Any + Send + Sync>>,
    /// Extensions to the error.
    #[serde(skip_serializing_if = "error_extensions_is_empty")]
    pub extensions: Option<ErrorExtensionValues>,
}

impl Debug for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("Error")
            .field("message", &self.message)
            .field("extensions", &self.extensions)
            .finish()
    }
}

impl PartialEq for Error {
    fn eq(&self, other: &Self) -> bool {
        self.message.eq(&other.message) && self.extensions.eq(&other.extensions)
    }
}

impl Error {
    /// Create an error from the given error message.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            source: None,
            extensions: None,
        }
    }

    /// Convert the error to a server error.
    #[must_use]
    pub fn into_server_error(self, pos: Pos) -> ServerError {
        ServerError {
            message: self.message,
            source: self.source,
            locations: vec![pos],
            path: Vec::new(),
            extensions: self.extensions,
        }
    }
}

impl<T: Display + Send + Sync + 'static> From<T> for Error {
    fn from(e: T) -> Self {
        Self {
            message: e.to_string(),
            source: None,
            extensions: None,
        }
    }
}

impl From<ResolverError> for Error {
    fn from(e: ResolverError) -> Self {
        Self {
            message: e.message,
            source: Some(e.error),
            extensions: None,
        }
    }
}

/// An alias for `Result<T, Error>`.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// An error parsing the request.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ParseRequestError {
    /// An IO error occurred.
    #[error("{0}")]
    Io(#[from] std::io::Error),

    /// The request's syntax was invalid.
    #[error("Invalid request: {0}")]
    InvalidRequest(Box<dyn std::error::Error + Send + Sync>),

    /// The request's files map was invalid.
    #[error("Invalid files map: {0}")]
    InvalidFilesMap(Box<dyn std::error::Error + Send + Sync>),

    /// The request's multipart data was invalid.
    #[error("Invalid multipart data")]
    InvalidMultipart(multer::Error),

    /// Missing "operators" part for multipart request.
    #[error("Missing \"operators\" part")]
    MissingOperatorsPart,

    /// Missing "map" part for multipart request.
    #[error("Missing \"map\" part")]
    MissingMapPart,

    /// It's not an upload operation
    #[error("It's not an upload operation")]
    NotUpload,

    /// Files were missing the request.
    #[error("Missing files")]
    MissingFiles,

    /// The request's payload is too large, and this server rejected it.
    #[error("Payload too large")]
    PayloadTooLarge,

    /// The request is a batch request, but the server does not support batch requests.
    #[error("Batch requests are not supported")]
    UnsupportedBatch,
}

impl From<multer::Error> for ParseRequestError {
    fn from(err: multer::Error) -> Self {
        match err {
            multer::Error::FieldSizeExceeded { .. } | multer::Error::StreamSizeExceeded { .. } => {
                ParseRequestError::PayloadTooLarge
            }
            _ => ParseRequestError::InvalidMultipart(err),
        }
    }
}

impl From<mime::FromStrError> for ParseRequestError {
    fn from(e: mime::FromStrError) -> Self {
        Self::InvalidRequest(Box::new(e))
    }
}

/// An error which can be extended into a `Error`.
pub trait ErrorExtensions: Sized {
    /// Convert the error to a `Error`.
    fn extend(self) -> Error;

    /// Add extensions to the error, using a callback to make the extensions.
    fn extend_with<C>(self, cb: C) -> Error
    where
        C: FnOnce(&Self, &mut ErrorExtensionValues),
    {
        let mut new_extensions = Default::default();
        cb(&self, &mut new_extensions);

        let Error {
            message,
            source,
            extensions,
        } = self.extend();

        let mut extensions = extensions.unwrap_or_default();
        extensions.0.extend(new_extensions.0);

        Error {
            message,
            source,
            extensions: Some(extensions),
        }
    }
}

impl ErrorExtensions for Error {
    fn extend(self) -> Error {
        self
    }
}

// implementing for &E instead of E gives the user the possibility to implement for E which does
// not conflict with this implementation acting as a fallback.
impl<E: Display> ErrorExtensions for &E {
    fn extend(self) -> Error {
        Error {
            message: self.to_string(),
            source: None,
            extensions: None,
        }
    }
}

impl ErrorExtensions for ResolverError {
    #[inline]
    fn extend(self) -> Error {
        Error::from(self)
    }
}

/// Extend a `Result`'s error value with [`ErrorExtensions`](trait.ErrorExtensions.html).
pub trait ResultExt<T, E>: Sized {
    /// Extend the error value of the result with the callback.
    fn extend_err<C>(self, cb: C) -> Result<T>
    where
        C: FnOnce(&E, &mut ErrorExtensionValues);

    /// Extend the result to a `Result`.
    fn extend(self) -> Result<T>;
}

// This is implemented on E and not &E which means it cannot be used on foreign types.
// (see example).
impl<T, E> ResultExt<T, E> for std::result::Result<T, E>
where
    E: ErrorExtensions + Send + Sync + 'static,
{
    fn extend_err<C>(self, cb: C) -> Result<T>
    where
        C: FnOnce(&E, &mut ErrorExtensionValues),
    {
        match self {
            Err(err) => Err(err.extend_with(|e, ee| cb(e, ee))),
            Ok(value) => Ok(value),
        }
    }

    fn extend(self) -> Result<T> {
        match self {
            Err(err) => Err(err.extend()),
            Ok(value) => Ok(value),
        }
    }
}

/// A wrapper around a dynamic error type for resolver.
#[derive(Debug)]
pub struct ResolverError {
    message: String,
    error: Arc<dyn Any + Send + Sync>,
}

impl<T: StdError + Send + Sync + 'static> From<T> for ResolverError {
    fn from(err: T) -> Self {
        Self {
            message: err.to_string(),
            error: Arc::new(err),
        }
    }
}

impl ResolverError {
    /// Create a new failure.
    #[inline]
    pub fn new<T: StdError + Send + Sync + 'static>(err: T) -> Self {
        From::from(err)
    }
}
