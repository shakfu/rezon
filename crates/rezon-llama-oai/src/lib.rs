//! OpenAI-compatible chat templating and streaming tool-call parsing for
//! `llama-cpp-2`, which dropped its `openai` module in 0.1.147.
//!
//! A thin C ABI over llama.cpp `common/chat.h` (`src/shim.cpp`). Results
//! cross the boundary as JSON.

use std::ffi::{c_char, c_int, CStr, CString};
use std::ptr::{self, NonNull};

use llama_cpp_2::model::{LlamaChatTemplate, LlamaModel};
use llama_cpp_sys_2::llama_model;
use serde::Deserialize;
use serde_json::Value;

#[repr(C)]
struct RawTemplateParams {
    messages: *const c_char,
    tools: *const c_char,
    tool_choice: *const c_char,
    json_schema: *const c_char,
    grammar: *const c_char,
    reasoning_format: *const c_char,
    chat_template_kwargs: *const c_char,
    add_generation_prompt: bool,
    use_jinja: bool,
    parallel_tool_calls: bool,
    enable_thinking: bool,
}

#[repr(C)]
struct RawParseState {
    _private: [u8; 0],
}

extern "C" {
    fn rezon_oai_string_free(s: *mut c_char);
    fn rezon_oai_apply_template(
        model: *const llama_model,
        chat_template: *const c_char,
        params: *const RawTemplateParams,
        out: *mut *mut c_char,
    ) -> c_int;
    fn rezon_oai_parse(
        chat_format: c_int,
        parse_tool_calls: bool,
        parser: *const c_char,
        generation_prompt: *const c_char,
        text: *const c_char,
        is_partial: bool,
        out: *mut *mut c_char,
    ) -> c_int;
    fn rezon_oai_parse_state_new(
        chat_format: c_int,
        parse_tool_calls: bool,
        parser: *const c_char,
        generation_prompt: *const c_char,
        out: *mut *mut c_char,
    ) -> *mut RawParseState;
    fn rezon_oai_parse_state_free(state: *mut RawParseState);
    fn rezon_oai_parse_state_update(
        state: *mut RawParseState,
        text_added: *const c_char,
        is_partial: bool,
        out: *mut *mut c_char,
    ) -> c_int;
}

/// Errors from templating or parsing.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("string contains a NUL byte: {0}")]
    Nul(#[from] std::ffi::NulError),
    #[error("llama.cpp: {0}")]
    Llama(String),
    #[error("shim returned no output")]
    NullResult,
    #[error("shim output: {0}")]
    Json(#[from] serde_json::Error),
}

/// Parameters for [`LlamaModelOaiExt::apply_chat_template_oaicompat`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OpenAIChatTemplateParams<'a> {
    /// OpenAI-compatible messages JSON array.
    pub messages_json: &'a str,
    /// OpenAI-compatible tools JSON array.
    pub tools_json: Option<&'a str>,
    /// `auto`, `required` or `none`.
    pub tool_choice: Option<&'a str>,
    /// JSON schema for constrained output.
    pub json_schema: Option<&'a str>,
    /// Custom GBNF grammar.
    pub grammar: Option<&'a str>,
    /// llama.cpp reasoning format name, e.g. `deepseek`.
    pub reasoning_format: Option<&'a str>,
    /// Extra template variables, as a JSON object.
    pub chat_template_kwargs: Option<&'a str>,
    pub add_generation_prompt: bool,
    pub use_jinja: bool,
    pub parallel_tool_calls: bool,
    pub enable_thinking: bool,
    /// Whether responses should be parsed for tool calls.
    pub parse_tool_calls: bool,
}

/// llama.cpp `common_grammar_trigger_type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(try_from = "i32")]
pub enum GrammarTriggerType {
    Token,
    Word,
    Pattern,
    PatternFull,
}

impl TryFrom<i32> for GrammarTriggerType {
    type Error = String;
    fn try_from(v: i32) -> Result<Self, String> {
        match v {
            0 => Ok(Self::Token),
            1 => Ok(Self::Word),
            2 => Ok(Self::Pattern),
            3 => Ok(Self::PatternFull),
            _ => Err(format!("unknown grammar trigger type {v}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct GrammarTrigger {
    #[serde(rename = "type")]
    pub trigger_type: GrammarTriggerType,
    pub value: String,
    /// Meaningful only for [`GrammarTriggerType::Token`].
    pub token: i32,
}

/// Rendered prompt plus what the parser and sampler need for the reply.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ChatTemplateResult {
    pub prompt: String,
    #[serde(deserialize_with = "empty_as_none")]
    pub grammar: Option<String>,
    pub grammar_lazy: bool,
    pub grammar_triggers: Vec<GrammarTrigger>,
    pub preserved_tokens: Vec<String>,
    pub additional_stops: Vec<String>,
    pub chat_format: i32,
    /// Serialized PEG parser; `None` when the template has no tool format.
    #[serde(deserialize_with = "empty_as_none")]
    pub parser: Option<String>,
    pub generation_prompt: String,
    #[serde(skip)]
    pub parse_tool_calls: bool,
}

fn empty_as_none<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    let s = String::deserialize(d)?;
    Ok((!s.is_empty()).then_some(s))
}

/// Takes ownership of a shim output string.
fn take_output(rc: c_int, out: *mut c_char) -> Result<String, Error> {
    if out.is_null() {
        return Err(Error::NullResult);
    }
    let s = unsafe { CStr::from_ptr(out) }
        .to_string_lossy()
        .into_owned();
    unsafe { rezon_oai_string_free(out) };
    if rc == 0 {
        Ok(s)
    } else {
        Err(Error::Llama(s))
    }
}

fn opt_cstring(s: Option<&str>) -> Result<Option<CString>, Error> {
    Ok(s.map(CString::new).transpose()?)
}

fn opt_ptr(s: &Option<CString>) -> *const c_char {
    s.as_ref().map_or(ptr::null(), |c| c.as_ptr())
}

/// The raw model pointer, which `llama-cpp-2` keeps `pub(crate)`.
fn model_ptr(model: &LlamaModel) -> *const llama_model {
    // SAFETY: in llama-cpp-2 0.1.160 (pinned with `=`), `LlamaModel` is
    // `#[repr(transparent)]` over `Ptr<llama_model>`, itself
    // `#[repr(transparent)]` over `NonNull<llama_model>`.
    const _: () = assert!(size_of::<LlamaModel>() == size_of::<NonNull<llama_model>>());
    unsafe { *ptr::from_ref(model).cast::<NonNull<llama_model>>() }.as_ptr()
}

/// OpenAI-compatible templating on [`LlamaModel`].
pub trait LlamaModelOaiExt {
    /// Render `params.messages_json` (and tools) with the model's Jinja template.
    fn apply_chat_template_oaicompat(
        &self,
        tmpl: &LlamaChatTemplate,
        params: &OpenAIChatTemplateParams<'_>,
    ) -> Result<ChatTemplateResult, Error>;
}

impl LlamaModelOaiExt for LlamaModel {
    fn apply_chat_template_oaicompat(
        &self,
        tmpl: &LlamaChatTemplate,
        params: &OpenAIChatTemplateParams<'_>,
    ) -> Result<ChatTemplateResult, Error> {
        let messages = CString::new(params.messages_json)?;
        let tools = opt_cstring(params.tools_json)?;
        let tool_choice = opt_cstring(params.tool_choice)?;
        let json_schema = opt_cstring(params.json_schema)?;
        let grammar = opt_cstring(params.grammar)?;
        let reasoning_format = opt_cstring(params.reasoning_format)?;
        let kwargs = opt_cstring(params.chat_template_kwargs)?;
        let raw = RawTemplateParams {
            messages: messages.as_ptr(),
            tools: opt_ptr(&tools),
            tool_choice: opt_ptr(&tool_choice),
            json_schema: opt_ptr(&json_schema),
            grammar: opt_ptr(&grammar),
            reasoning_format: opt_ptr(&reasoning_format),
            chat_template_kwargs: opt_ptr(&kwargs),
            add_generation_prompt: params.add_generation_prompt,
            use_jinja: params.use_jinja,
            parallel_tool_calls: params.parallel_tool_calls,
            enable_thinking: params.enable_thinking,
        };
        let mut out = ptr::null_mut();
        let rc = unsafe {
            rezon_oai_apply_template(model_ptr(self), tmpl.as_c_str().as_ptr(), &raw, &mut out)
        };
        let mut result: ChatTemplateResult = serde_json::from_str(&take_output(rc, out)?)?;
        result.parse_tool_calls = params.parse_tool_calls;
        Ok(result)
    }
}

impl ChatTemplateResult {
    fn parser_args(&self) -> Result<(Option<CString>, Option<CString>), Error> {
        let gen_prompt = (!self.generation_prompt.is_empty()).then_some(&*self.generation_prompt);
        Ok((
            opt_cstring(self.parser.as_deref())?,
            opt_cstring(gen_prompt)?,
        ))
    }

    /// Parse a complete (or partial) response into an OpenAI message object.
    pub fn parse_response_oaicompat(&self, text: &str, is_partial: bool) -> Result<Value, Error> {
        let (parser, gen_prompt) = self.parser_args()?;
        let text = CString::new(text)?;
        let mut out = ptr::null_mut();
        let rc = unsafe {
            rezon_oai_parse(
                self.chat_format,
                self.parse_tool_calls,
                opt_ptr(&parser),
                opt_ptr(&gen_prompt),
                text.as_ptr(),
                is_partial,
                &mut out,
            )
        };
        Ok(serde_json::from_str(&take_output(rc, out)?)?)
    }

    /// Start a streaming parser that turns generated text into OpenAI deltas.
    pub fn streaming_state_oaicompat(&self) -> Result<ChatParseStateOaicompat, Error> {
        let (parser, gen_prompt) = self.parser_args()?;
        let mut out = ptr::null_mut();
        let state = unsafe {
            rezon_oai_parse_state_new(
                self.chat_format,
                self.parse_tool_calls,
                opt_ptr(&parser),
                opt_ptr(&gen_prompt),
                &mut out,
            )
        };
        match NonNull::new(state) {
            Some(state) => Ok(ChatParseStateOaicompat { state }),
            None => Err(take_output(1, out).err().unwrap_or(Error::NullResult)),
        }
    }
}

/// Incremental parser; see [`ChatTemplateResult::streaming_state_oaicompat`].
#[derive(Debug)]
pub struct ChatParseStateOaicompat {
    state: NonNull<RawParseState>,
}

impl ChatParseStateOaicompat {
    /// Append `text_added` and return the OpenAI `delta` objects it produced.
    pub fn update(&mut self, text_added: &str, is_partial: bool) -> Result<Vec<Value>, Error> {
        let text = CString::new(text_added)?;
        let mut out = ptr::null_mut();
        let rc = unsafe {
            rezon_oai_parse_state_update(self.state.as_ptr(), text.as_ptr(), is_partial, &mut out)
        };
        Ok(serde_json::from_str(&take_output(rc, out)?)?)
    }
}

impl Drop for ChatParseStateOaicompat {
    fn drop(&mut self) {
        unsafe { rezon_oai_parse_state_free(self.state.as_ptr()) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn content_only() -> ChatTemplateResult {
        ChatTemplateResult {
            prompt: String::new(),
            grammar: None,
            grammar_lazy: false,
            grammar_triggers: Vec::new(),
            preserved_tokens: Vec::new(),
            additional_stops: Vec::new(),
            chat_format: 0,
            parser: None,
            generation_prompt: String::new(),
            parse_tool_calls: false,
        }
    }

    #[test]
    fn template_result_maps_empty_strings_to_none() {
        let json = r#"{"prompt":"p","grammar":"","grammar_lazy":true,
            "grammar_triggers":[{"type":1,"value":"<tool_call>","token":-1}],
            "preserved_tokens":["<tool_call>"],"additional_stops":[],
            "chat_format":2,"parser":"","generation_prompt":"<|im_start|>assistant\n"}"#;
        let r: ChatTemplateResult = serde_json::from_str(json).unwrap();
        assert_eq!(r.grammar, None);
        assert_eq!(r.parser, None);
        assert_eq!(r.grammar_triggers[0].trigger_type, GrammarTriggerType::Word);
        assert_eq!(r.chat_format, 2);
    }

    #[test]
    fn unknown_trigger_type_is_rejected() {
        let json = r#"{"type":9,"value":"","token":0}"#;
        assert!(serde_json::from_str::<GrammarTrigger>(json).is_err());
    }

    // Exercises the C++ shim and llama.cpp's parser without a model.
    #[test]
    fn streaming_content_only_yields_content_deltas() {
        let mut state = content_only().streaming_state_oaicompat().unwrap();
        let mut text = String::new();
        for piece in ["Hel", "lo, ", "world"] {
            for d in state.update(piece, true).unwrap() {
                text.push_str(d["content"].as_str().unwrap());
            }
        }
        assert_eq!(text, "Hello, world");
    }

    #[test]
    fn parse_response_returns_assistant_message() {
        let msg = content_only()
            .parse_response_oaicompat("plain answer", false)
            .unwrap();
        assert_eq!(msg["role"], "assistant");
        assert_eq!(msg["content"], "plain answer");
    }

    #[test]
    fn invalid_parser_reports_llama_error() {
        let mut r = content_only();
        r.parser = Some("not a parser".into());
        assert!(matches!(
            r.streaming_state_oaicompat(),
            Err(Error::Llama(_))
        ));
    }
}
