// OpenAI-compatible chat templating and streaming tool-call parsing over
// llama.cpp `common/chat.h`. Replaces the `wrapper_oai.cpp` shim that
// llama-cpp-sys-2 dropped in 0.1.147.
//
// Every fallible call returns 0 on success and stores a malloc'd string in
// `*out`: the JSON result on success, the exception message on failure.
// Release it with `rezon_oai_string_free`.

#include <cstdlib>
#include <cstring>
#include <exception>
#include <random>
#include <string>
#include <vector>

#include "chat.h"
#include "llama.h"

struct rezon_oai_template_params {
    const char * messages;
    const char * tools;
    const char * tool_choice;
    const char * json_schema;
    const char * grammar;
    const char * reasoning_format;
    const char * chat_template_kwargs;
    bool         add_generation_prompt;
    bool         use_jinja;
    bool         parallel_tool_calls;
    bool         enable_thinking;
};

struct rezon_oai_parse_state {
    common_chat_parser_params syntax;
    common_chat_msg           msg;
    std::string               text;
    std::vector<std::string>  tool_call_ids;
};

static char * dup_string(const std::string & s) {
    char * p = static_cast<char *>(std::malloc(s.size() + 1));
    if (p) {
        std::memcpy(p, s.c_str(), s.size() + 1);
    }
    return p;
}

static int finish(int rc, const std::string & s, char ** out) {
    *out = dup_string(s);
    return *out ? rc : 2;
}

static bool nonempty(const char * s) {
    return s && s[0] != '\0';
}

static std::string random_string(size_t length = 32) {
    static constexpr char chars[] = "0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
    static thread_local std::mt19937 gen{ std::random_device{}() };
    std::uniform_int_distribution<size_t> dist(0, sizeof(chars) - 2);
    std::string result(length, '\0');
    for (auto & c : result) {
        c = chars[dist(gen)];
    }
    return result;
}

static common_json string_array(const std::vector<std::string> & items) {
    auto arr = common_json::array();
    for (const auto & s : items) {
        arr.push_back(s);
    }
    return arr;
}

// llama.cpp dropped this from common/chat in favour of server-side code.
static common_json diff_to_json(const common_chat_msg_diff & diff) {
    auto delta = common_json::object();
    if (!diff.reasoning_content_delta.empty()) {
        delta["reasoning_content"] = diff.reasoning_content_delta;
    }
    if (!diff.content_delta.empty()) {
        delta["content"] = diff.content_delta;
    }
    if (diff.tool_call_index != std::string::npos) {
        auto tool_call     = common_json::object();
        tool_call["index"] = diff.tool_call_index;
        if (!diff.tool_call_delta.id.empty()) {
            tool_call["id"]   = diff.tool_call_delta.id;
            tool_call["type"] = "function";
        }
        if (!diff.tool_call_delta.name.empty() || !diff.tool_call_delta.arguments.empty()) {
            auto function = common_json::object();
            if (!diff.tool_call_delta.name.empty()) {
                function["name"] = diff.tool_call_delta.name;
            }
            if (!diff.tool_call_delta.arguments.empty()) {
                function["arguments"] = diff.tool_call_delta.arguments;
            }
            tool_call["function"] = function;
        }
        auto calls = common_json::array();
        calls.push_back(tool_call);
        delta["tool_calls"] = calls;
    }
    return delta;
}

static common_chat_parser_params make_syntax(int chat_format, bool parse_tool_calls, const char * parser,
                                             const char * generation_prompt) {
    common_chat_parser_params syntax;
    syntax.format           = static_cast<common_chat_format>(chat_format);
    syntax.parse_tool_calls = parse_tool_calls;
    if (nonempty(generation_prompt)) {
        syntax.generation_prompt = generation_prompt;
    }
    if (nonempty(parser)) {
        syntax.parser.load(parser);
    }
    return syntax;
}

extern "C" {

void rezon_oai_string_free(char * s) {
    std::free(s);
}

int rezon_oai_apply_template(const llama_model * model, const char * chat_template,
                             const rezon_oai_template_params * params, char ** out) {
    *out = nullptr;
    if (!model || !chat_template || !params || !params->messages) {
        return finish(1, "null argument", out);
    }
    try {
        auto tmpls = common_chat_templates_init(model, chat_template);

        common_chat_templates_inputs inputs;
        inputs.add_generation_prompt = params->add_generation_prompt;
        inputs.use_jinja             = params->use_jinja;
        inputs.parallel_tool_calls   = params->parallel_tool_calls;
        inputs.enable_thinking       = params->enable_thinking;
        inputs.messages              = common_chat_msgs_parse_oaicompat(common_json::parse(params->messages));
        if (nonempty(params->tools)) {
            inputs.tools = common_chat_tools_parse_oaicompat(common_json::parse(params->tools));
        }
        if (nonempty(params->tool_choice)) {
            inputs.tool_choice = common_chat_tool_choice_parse_oaicompat(params->tool_choice);
        }
        if (nonempty(params->json_schema)) {
            inputs.json_schema = params->json_schema;
        }
        if (nonempty(params->grammar)) {
            inputs.grammar = params->grammar;
        }
        if (nonempty(params->reasoning_format)) {
            inputs.reasoning_format = common_reasoning_format_from_name(params->reasoning_format);
        }
        if (nonempty(params->chat_template_kwargs)) {
            auto kwargs = common_json::parse(params->chat_template_kwargs);
            if (!kwargs.is_object()) {
                return finish(1, "chat_template_kwargs must be a JSON object", out);
            }
            for (const auto & [key, val] : kwargs.items()) {
                inputs.chat_template_kwargs[key] = val.dump();
            }
        }

        auto r = common_chat_templates_apply(tmpls.get(), inputs);

        auto triggers = common_json::array();
        for (const auto & t : r.grammar_triggers) {
            auto trig     = common_json::object();
            trig["type"]  = static_cast<int>(t.type);
            trig["value"] = t.value;
            trig["token"] = t.token;
            triggers.push_back(trig);
        }

        auto res                 = common_json::object();
        res["prompt"]            = r.prompt;
        res["grammar"]           = r.grammar;
        res["grammar_lazy"]      = r.grammar_lazy;
        res["grammar_triggers"]  = triggers;
        res["preserved_tokens"]  = string_array(r.preserved_tokens);
        res["additional_stops"]  = string_array(r.additional_stops);
        res["chat_format"]       = static_cast<int>(r.format);
        res["parser"]            = r.parser;
        res["generation_prompt"] = r.generation_prompt;
        return finish(0, res.dump(), out);
    } catch (const std::exception & e) {
        return finish(1, e.what(), out);
    }
}

int rezon_oai_parse(int chat_format, bool parse_tool_calls, const char * parser, const char * generation_prompt,
                    const char * text, bool is_partial, char ** out) {
    *out = nullptr;
    if (!text) {
        return finish(1, "null argument", out);
    }
    try {
        auto syntax = make_syntax(chat_format, parse_tool_calls, parser, generation_prompt);
        auto msg    = common_chat_parse(text, is_partial, syntax);
        std::vector<std::string> ids;
        msg.set_tool_call_ids(ids, [] { return random_string(); });
        return finish(0, msg.to_json_oaicompat().dump(), out);
    } catch (const std::exception & e) {
        return finish(1, e.what(), out);
    }
}

rezon_oai_parse_state * rezon_oai_parse_state_new(int chat_format, bool parse_tool_calls, const char * parser,
                                                  const char * generation_prompt, char ** out) {
    *out = nullptr;
    try {
        return new rezon_oai_parse_state{ make_syntax(chat_format, parse_tool_calls, parser, generation_prompt),
                                          {}, {}, {} };
    } catch (const std::exception & e) {
        finish(1, e.what(), out);
        return nullptr;
    }
}

void rezon_oai_parse_state_free(rezon_oai_parse_state * state) {
    delete state;
}

// `*out` receives a JSON array of OpenAI `delta` objects, one per diff.
int rezon_oai_parse_state_update(rezon_oai_parse_state * state, const char * text_added, bool is_partial,
                                 char ** out) {
    *out = nullptr;
    if (!state || !text_added) {
        return finish(1, "null argument", out);
    }
    try {
        state->text += text_added;
        auto deltas  = common_json::array();
        auto new_msg = common_chat_parse(state->text, is_partial, state->syntax);
        if (!new_msg.empty()) {
            new_msg.set_tool_call_ids(state->tool_call_ids, [] { return random_string(); });
            for (const auto & d : common_chat_msg_diff::compute_diffs(state->msg, new_msg)) {
                deltas.push_back(diff_to_json(d));
            }
            state->msg = std::move(new_msg);
        }
        return finish(0, deltas.dump(), out);
    } catch (const std::exception & e) {
        return finish(1, e.what(), out);
    }
}

}  // extern "C"
