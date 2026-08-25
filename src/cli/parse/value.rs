use crate::error::FormatError;

pub(super) struct ArgCursor<I> {
    inner: I,
}

impl<I> ArgCursor<I>
where
    I: Iterator<Item = String>,
{
    pub(super) fn new(inner: I) -> Self {
        Self { inner }
    }

    pub(super) fn next(&mut self) -> Option<String> {
        self.inner.next()
    }

    /// Required long-option values may be attached with `=` or consume the
    /// next argv element, including one that starts with `-`.
    pub(super) fn required_long(
        &mut self,
        inline: &mut Option<String>,
    ) -> Result<String, FormatError> {
        if let Some(value) = inline.take() {
            Ok(value)
        } else {
            self.next()
                .ok_or_else(|| FormatError::InvalidOption("missing option value".into()))
        }
    }

    /// Findent short options accept attached values (`-i4`) and separated
    /// values (`-i 4`). Optional long values deliberately do not use this.
    pub(super) fn required_short(
        &mut self,
        option: char,
        attached: &str,
    ) -> Result<String, FormatError> {
        if attached.is_empty() {
            self.next()
                .ok_or_else(|| FormatError::InvalidOption(format!("-{option} requires a value")))
        } else {
            Ok(attached.to_string())
        }
    }
}
