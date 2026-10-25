#include "cef-openmp/src/lib.rs.h"

#ifndef SAMP_CEF_HAS_GRPC_EXT
#define SAMP_CEF_HAS_GRPC_EXT 0
#endif

#if SAMP_CEF_HAS_GRPC_EXT
#include <omp_ext/component.hpp>
#endif

#include <Server/Components/Pawn/pawn.hpp>
#include <amx/amx.h>
#include <sdk.hpp>

#include <algorithm>
#include <cstdint>
#include <optional>
#include <set>
#include <string>
#include <unordered_map>
#include <utility>
#include <vector>

namespace
{
using samp_cef::openmp::EventArguments;
using samp_cef::openmp::EventKind;
using samp_cef::openmp::ServerCore;
using samp_cef::openmp::ServerEvent;

constexpr int CEF_DEFAULT_PORT_OFFSET = 2;
constexpr float CEF_DEFAULT_MAX_DIST = 50.0f;
constexpr float CEF_DEFAULT_REF_DIST = 15.0f;
constexpr const char* CEF_GRPC_BROWSER_EVENT_PREFIX = "cef.event.";
constexpr UID CEF_COMPONENT_UID = UID(0xCEF5A17CEFC0FFEEull);

rust::Str asRustStr(const std::string& value)
{
	return rust::Str(value.data(), value.size());
}

std::string toStdString(const rust::String& value)
{
	return std::string(value.data(), value.size());
}

#if SAMP_CEF_HAS_GRPC_EXT
using GrpcRequest = omp_ext::IOmpGrpcComponent::InvokeRequest;
using GrpcReply = omp_ext::IOmpGrpcComponent::InvokeReply;
using GrpcNamedValue = omp_ext::IOmpGrpcComponent::NamedValue;
using GrpcFunctionDescriptor = omp_ext::IOmpGrpcComponent::FunctionDescriptor;
using GrpcEventDescriptor = omp_ext::IOmpGrpcComponent::EventDescriptor;

const omp::ext::v1::Value* grpcArg(const GrpcRequest& request, const char* name, int index)
{
	for (const auto& arg : request.args())
	{
		if (arg.name() == name)
		{
			return &arg.value();
		}
	}

	if (index >= 0 && index < request.args_size())
	{
		return &request.args(index).value();
	}
	return nullptr;
}

void grpcFail(GrpcReply& reply, const char* code, const std::string& message)
{
	auto* error = reply.mutable_error();
	error->set_code(code);
	error->set_message(message);
}

bool grpcReadIntValue(const omp::ext::v1::Value& value, int& output)
{
	if (value.has_int32_value())
	{
		output = value.int32_value();
		return true;
	}
	if (value.has_uint32_value())
	{
		output = static_cast<int>(value.uint32_value());
		return true;
	}
	if (value.has_int64_value())
	{
		output = static_cast<int>(value.int64_value());
		return true;
	}
	if (value.has_uint64_value())
	{
		output = static_cast<int>(value.uint64_value());
		return true;
	}
	return false;
}

bool grpcReadPlayerValue(const omp::ext::v1::Value& value, int& output)
{
	if (value.has_entity_value())
	{
		const auto& entity = value.entity_value();
		if (entity.type() == omp::ext::v1::ENTITY_TYPE_PLAYER || entity.type() == omp::ext::v1::ENTITY_TYPE_UNSPECIFIED)
		{
			output = entity.id();
			return true;
		}
		return false;
	}
	return grpcReadIntValue(value, output);
}

bool grpcReadUInt32Value(const omp::ext::v1::Value& value, std::uint32_t& output)
{
	int integer = 0;
	if (!grpcReadIntValue(value, integer))
	{
		return false;
	}
	output = static_cast<std::uint32_t>(integer);
	return true;
}

bool grpcReadBoolValue(const omp::ext::v1::Value& value, bool& output)
{
	if (value.has_bool_value())
	{
		output = value.bool_value();
		return true;
	}
	int integer = 0;
	if (grpcReadIntValue(value, integer))
	{
		output = integer != 0;
		return true;
	}
	return false;
}

bool grpcReadFloatValue(const omp::ext::v1::Value& value, float& output)
{
	if (value.has_float_value())
	{
		output = value.float_value();
		return true;
	}
	if (value.has_double_value())
	{
		output = static_cast<float>(value.double_value());
		return true;
	}
	int integer = 0;
	if (grpcReadIntValue(value, integer))
	{
		output = static_cast<float>(integer);
		return true;
	}
	return false;
}

bool grpcReadStringValue(const omp::ext::v1::Value& value, std::string& output)
{
	if (!value.has_string_value())
	{
		return false;
	}
	output = value.string_value();
	return true;
}

template <typename Reader, typename T>
bool grpcReadRequired(const GrpcRequest& request, GrpcReply& reply, const char* name, int index, Reader reader, T& output, const char* expected)
{
	const auto* value = grpcArg(request, name, index);
	if (!value || !reader(*value, output))
	{
		grpcFail(reply, "BAD_ARGUMENT", std::string("Expected ") + expected + " argument `" + name + "`");
		return false;
	}
	return true;
}

template <typename Reader, typename T>
bool grpcReadOptional(const GrpcRequest& request, const char* name, int index, Reader reader, T& output)
{
	const auto* value = grpcArg(request, name, index);
	return value ? reader(*value, output) : true;
}

void grpcReturnBool(GrpcReply& reply, bool value)
{
	reply.mutable_return_value()->set_bool_value(value);
}

GrpcNamedValue grpcNamedPlayer(const char* name, int playerID)
{
	GrpcNamedValue arg;
	arg.set_name(name);
	auto* entity = arg.mutable_value()->mutable_entity_value();
	entity->set_type(omp::ext::v1::ENTITY_TYPE_PLAYER);
	entity->set_id(playerID);
	return arg;
}

GrpcNamedValue grpcNamedInt(const char* name, int value)
{
	GrpcNamedValue arg;
	arg.set_name(name);
	arg.mutable_value()->set_int32_value(value);
	return arg;
}

GrpcNamedValue grpcNamedBool(const char* name, bool value)
{
	GrpcNamedValue arg;
	arg.set_name(name);
	arg.mutable_value()->set_bool_value(value);
	return arg;
}

GrpcNamedValue grpcNamedString(const char* name, const std::string& value)
{
	GrpcNamedValue arg;
	arg.set_name(name);
	arg.mutable_value()->set_string_value(value);
	return arg;
}

void addGrpcParam(GrpcFunctionDescriptor& descriptor, const char* name, const char* type,
	omp::ext::v1::EntityType entityType = omp::ext::v1::ENTITY_TYPE_UNSPECIFIED)
{
	omp::ext::v1::NamedType param;
	param.set_name(name);
	param.set_type(type);
	param.set_entity_type(entityType);
	descriptor.params.push_back(param);
}

void addGrpcEventArg(GrpcEventDescriptor& descriptor, const char* name, const char* type,
	omp::ext::v1::EntityType entityType = omp::ext::v1::ENTITY_TYPE_UNSPECIFIED)
{
	descriptor.args.push_back(omp_ext::EventArgDescriptor { name, type, entityType });
}

std::string grpcBrowserEventName(const std::string& name)
{
	return std::string(CEF_GRPC_BROWSER_EVENT_PREFIX) + name;
}
#endif

int pawnParamCount(const cell* params)
{
	return params ? static_cast<int>(params[0] / static_cast<cell>(sizeof(cell))) : 0;
}

class CefComponent final : public IComponent, public CoreEventHandler, public PlayerConnectEventHandler, public PawnEventHandler
{
public:
	PROVIDE_UID(CEF_COMPONENT_UID);

	StringView componentName() const override
	{
		return "CEF";
	}

	SemanticVersion componentVersion() const override
	{
		return SemanticVersion(OMP_VERSION_MAJOR, OMP_VERSION_MINOR, OMP_VERSION_PATCH, BUILD_NUMBER);
	}

	void provideConfiguration(ILogger&, IEarlyConfig& config, bool defaults) override
	{
		if (defaults)
		{
			config.setInt("cef.port_offset", CEF_DEFAULT_PORT_OFFSET);
		}
	}

	void onLoad(ICore* c) override
	{
		core_ = c;
		instance_ = this;

		core_->getEventDispatcher().addEventHandler(this);
		core_->getPlayers().getPlayerConnectDispatcher().addEventHandler(this);

		IConfig& config = core_->getConfig();
		StringView bindView = config.getString("network.bind");
		std::string bind(bindView.data(), bindView.length());
		if (bind.empty())
		{
			bind = "0.0.0.0";
		}

		int port = 7777;
		if (int* configuredPort = config.getInt("network.port"))
		{
			port = *configuredPort;
		}

		int portOffset = CEF_DEFAULT_PORT_OFFSET;
		if (int* configuredOffset = config.getInt("cef.port_offset"))
		{
			portOffset = *configuredOffset;
		}

		const int cefPort = port + portOffset;
		if (cefPort < 0 || cefPort > 65535)
		{
			core_->logLn(LogLevel::Error, "[CEF] Invalid CEF server port: %d", cefPort);
			return;
		}

		server_.emplace(samp_cef::openmp::create_server_core(asRustStr(bind), static_cast<std::uint16_t>(cefPort)));
		if ((*server_)->is_running())
		{
			core_->printLn("[CEF] Bound CEF server on %s:%d", bind.c_str(), cefPort);
		}
		else
		{
			const std::string error = toStdString((*server_)->last_error());
			core_->logLn(LogLevel::Error, "[CEF] %s", error.c_str());
		}
	}

	void onInit(IComponentList* components) override
	{
		pawn_ = components->queryComponent<IPawnComponent>();
		if (pawn_)
		{
			pawn_->getEventDispatcher().addEventHandler(this);
		}

#if SAMP_CEF_HAS_GRPC_EXT
		grpc_ = components->queryComponent<omp_ext::IOmpGrpcComponent>();
		if (grpc_)
		{
			registerGrpcApi();
		}
#endif
	}

	void onFree(IComponent* component) override
	{
		if (component == pawn_ && pawn_)
		{
			pawn_->getEventDispatcher().removeEventHandler(this);
			pawn_ = nullptr;
		}

#if SAMP_CEF_HAS_GRPC_EXT
		if (component == grpc_ && grpc_)
		{
			grpc_->unregisterOwner(this);
			grpc_ = nullptr;
			grpcRegisteredEvents_.clear();
			grpcBrowserEvents_.clear();
		}
#endif
	}

	void free() override
	{
		shutdown();
		delete this;
	}

	void reset() override
	{
		scripts_.clear();
		subscriptions_.clear();
	}

	void onTick(Microseconds, TimePoint) override
	{
		if (!server_ || !(*server_)->is_running())
		{
			return;
		}

		for (;;)
		{
			ServerEvent event = (*server_)->poll_event();
			if (event.kind == EventKind::None)
			{
				break;
			}

			dispatchEvent(event);
		}
	}

	void onIncomingConnection(IPlayer& player, StringView ipAddress, unsigned short) override
	{
		if (!server_ || !(*server_)->is_running())
		{
			return;
		}

		const std::string ip(ipAddress.data(), ipAddress.length());
		if (!(*server_)->allow_connection(player.getID(), asRustStr(ip)))
		{
			core_->logLn(LogLevel::Warning, "[CEF] Invalid incoming player IP: %s", ip.c_str());
		}
	}

	void onPlayerDisconnect(IPlayer& player, PeerDisconnectReason) override
	{
		if (server_)
		{
			(*server_)->remove_connection(player.getID());
		}
	}

	void onAmxLoad(IPawnScript& script) override
	{
		static AMX_NATIVE_INFO natives[] = {
			{ "cef_on_player_connect", &CefComponent::nativeOnPlayerConnect },
			{ "cef_on_player_disconnect", &CefComponent::nativeOnPlayerDisconnect },
			{ "cef_create_browser", &CefComponent::nativeCreateBrowser },
			{ "cef_destroy_browser", &CefComponent::nativeDestroyBrowser },
			{ "cef_emit_event", &CefComponent::nativeEmitEvent },
			{ "cef_subscribe", &CefComponent::nativeSubscribe },
			{ "cef_always_listen_keys", &CefComponent::nativeAlwaysListenKeys },
			{ "cef_hide_browser", &CefComponent::nativeHideBrowser },
			{ "cef_focus_browser", &CefComponent::nativeFocusBrowser },
			{ "cef_player_has_plugin", &CefComponent::nativePlayerHasPlugin },
			{ "cef_create_ext_browser", &CefComponent::nativeCreateExternalBrowser },
			{ "cef_append_to_object", &CefComponent::nativeAppendToObject },
			{ "cef_remove_from_object", &CefComponent::nativeRemoveFromObject },
			{ "cef_toggle_dev_tools", &CefComponent::nativeToggleDevTools },
			{ "cef_set_audio_settings", &CefComponent::nativeSetAudioSettings },
			{ "cef_load_url", &CefComponent::nativeLoadUrl },
		};

		script.Register(natives, static_cast<int>(sizeof(natives) / sizeof(natives[0])));

		if (std::find(scripts_.begin(), scripts_.end(), &script) == scripts_.end())
		{
			scripts_.push_back(&script);
		}
	}

	void onAmxUnload(IPawnScript& script) override
	{
		scripts_.erase(std::remove(scripts_.begin(), scripts_.end(), &script), scripts_.end());

		for (auto it = subscriptions_.begin(); it != subscriptions_.end();)
		{
			if (it->second.script == &script)
			{
				it = subscriptions_.erase(it);
			}
			else
			{
				++it;
			}
		}
	}

private:
	struct Subscription
	{
		IPawnScript* script = nullptr;
		std::string callback;
	};

	ICore* core_ = nullptr;
	IPawnComponent* pawn_ = nullptr;
#if SAMP_CEF_HAS_GRPC_EXT
	omp_ext::IOmpGrpcComponent* grpc_ = nullptr;
	std::set<std::string> grpcRegisteredEvents_;
	std::set<std::string> grpcBrowserEvents_;
#endif
	std::optional<rust::Box<ServerCore>> server_;
	std::vector<IPawnScript*> scripts_;
	std::unordered_map<std::string, Subscription> subscriptions_;

	static CefComponent* instance_;

	void shutdown()
	{
#if SAMP_CEF_HAS_GRPC_EXT
		if (grpc_)
		{
			grpc_->unregisterOwner(this);
			grpc_ = nullptr;
		}
		grpcRegisteredEvents_.clear();
		grpcBrowserEvents_.clear();
#endif

		if (pawn_)
		{
			pawn_->getEventDispatcher().removeEventHandler(this);
			pawn_ = nullptr;
		}

		if (core_)
		{
			core_->getPlayers().getPlayerConnectDispatcher().removeEventHandler(this);
			core_->getEventDispatcher().removeEventHandler(this);
			core_ = nullptr;
		}

		scripts_.clear();
		subscriptions_.clear();
		server_.reset();

		if (instance_ == this)
		{
			instance_ = nullptr;
		}
	}

	void dispatchEvent(const ServerEvent& event)
	{
		switch (event.kind)
		{
		case EventKind::EmitEvent:
			dispatchBrowserEvent(event);
			break;

		case EventKind::PlayerInitialized:
			notifyConnect(event.player_id, event.success);
			break;

		case EventKind::BrowserCreated:
			notifyBrowserCreated(event.player_id, event.browser_id, event.code);
			break;

		case EventKind::None:
			break;
		}
	}

	void dispatchBrowserEvent(const ServerEvent& event)
	{
		const std::string name = toStdString(event.event);
		auto subscription = subscriptions_.find(name);
		const std::string arguments = toStdString(event.arguments);
		if (subscription != subscriptions_.end() && subscription->second.script)
		{
			subscription->second.script->Call(
				subscription->second.callback.c_str(),
				DefaultReturnValue_False,
				event.player_id,
				StringView(arguments.data(), arguments.length()));
		}

#if SAMP_CEF_HAS_GRPC_EXT
		emitGrpcBrowserEvent(name, event.player_id, arguments);
#endif
	}

	void notifyConnect(int playerID, bool success)
	{
		for (IPawnScript* script : scripts_)
		{
			script->Call("OnCefInitialize", DefaultReturnValue_True, playerID, success ? 1 : 0);
		}

#if SAMP_CEF_HAS_GRPC_EXT
		std::vector<GrpcNamedValue> args;
		args.push_back(grpcNamedPlayer("player_id", playerID));
		args.push_back(grpcNamedBool("success", success));
		emitGrpcEvent("OnCefInitialize", std::move(args));
#endif
	}

	void notifyBrowserCreated(int playerID, std::uint32_t browserID, int code)
	{
		for (IPawnScript* script : scripts_)
		{
			script->Call("OnCefBrowserCreated", DefaultReturnValue_True, playerID, static_cast<int>(browserID), code);
		}

#if SAMP_CEF_HAS_GRPC_EXT
		std::vector<GrpcNamedValue> args;
		args.push_back(grpcNamedPlayer("player_id", playerID));
		args.push_back(grpcNamedInt("browser_id", static_cast<int>(browserID)));
		args.push_back(grpcNamedInt("status_code", code));
		emitGrpcEvent("OnCefBrowserCreated", std::move(args));
#endif
	}

	IPawnScript* scriptFromAmx(AMX* amx) const
	{
		return pawn_ ? pawn_->getScript(amx) : nullptr;
	}

	bool getAmxString(IPawnScript& script, cell address, std::string& output) const
	{
		cell* physicalAddress = nullptr;
		if (script.GetAddr(address, &physicalAddress) != AMX_ERR_NONE || physicalAddress == nullptr)
		{
			return false;
		}

		int length = 0;
		if (script.StrLen(physicalAddress, &length) != AMX_ERR_NONE)
		{
			return false;
		}

		std::vector<char> buffer(static_cast<std::size_t>(length) + 1, '\0');
		if (script.GetString(buffer.data(), physicalAddress, false, buffer.size()) != AMX_ERR_NONE)
		{
			return false;
		}

		output.assign(buffer.data());
		return true;
	}

	bool getAmxCell(IPawnScript& script, cell address, cell& output) const
	{
		cell* physicalAddress = nullptr;
		if (script.GetAddr(address, &physicalAddress) != AMX_ERR_NONE || physicalAddress == nullptr)
		{
			return false;
		}

		output = *physicalAddress;
		return true;
	}

	bool requireParams(const cell* params, int count, const char* native) const
	{
		if (pawnParamCount(params) >= count)
		{
			return true;
		}

		if (core_)
		{
			core_->logLn(LogLevel::Error, "[CEF] Incorrect parameters for `%s`", native);
		}
		return false;
	}

#if SAMP_CEF_HAS_GRPC_EXT
	template <typename Init>
	void registerGrpcFunction(const char* name, const char* returnType, Init init, omp_ext::IOmpGrpcComponent::FunctionHandler handler)
	{
		GrpcFunctionDescriptor descriptor;
		descriptor.name = name;
		descriptor.category = "CEF";
		descriptor.returnType = returnType;
		descriptor.requiresCapi = false;
		init(descriptor);

		if (!grpc_->registerFunction(this, std::move(descriptor), handler, this) && core_)
		{
			core_->logLn(LogLevel::Warning, "[CEF] Failed to register gRPC function `%s`", name);
		}
	}

	template <typename Init>
	bool registerGrpcEvent(const std::string& name, Init init)
	{
		if (!grpc_)
		{
			return false;
		}

		if (grpcRegisteredEvents_.find(name) != grpcRegisteredEvents_.end())
		{
			return true;
		}

		GrpcEventDescriptor descriptor;
		descriptor.category = "CEF";
		descriptor.name = name;
		init(descriptor);

		if (!grpc_->registerEvent(this, std::move(descriptor)))
		{
			return false;
		}

		grpcRegisteredEvents_.insert(name);
		return true;
	}

	bool registerGrpcBrowserEvent(const std::string& name)
	{
		if (name.empty())
		{
			return false;
		}

		if (grpcBrowserEvents_.find(name) != grpcBrowserEvents_.end())
		{
			return true;
		}

		const bool registered = registerGrpcEvent(grpcBrowserEventName(name),
			[](GrpcEventDescriptor& descriptor)
			{
				addGrpcEventArg(descriptor, "player_id", "int", omp::ext::v1::ENTITY_TYPE_PLAYER);
				addGrpcEventArg(descriptor, "event", "char*");
				addGrpcEventArg(descriptor, "arguments", "char*");
			});
		if (registered)
		{
			grpcBrowserEvents_.insert(name);
		}
		return registered;
	}

	void registerGrpcApi()
	{
		registerGrpcEvent("OnCefInitialize",
			[](GrpcEventDescriptor& descriptor)
			{
				addGrpcEventArg(descriptor, "player_id", "int", omp::ext::v1::ENTITY_TYPE_PLAYER);
				addGrpcEventArg(descriptor, "success", "bool");
			});

		registerGrpcEvent("OnCefBrowserCreated",
			[](GrpcEventDescriptor& descriptor)
			{
				addGrpcEventArg(descriptor, "player_id", "int", omp::ext::v1::ENTITY_TYPE_PLAYER);
				addGrpcEventArg(descriptor, "browser_id", "int");
				addGrpcEventArg(descriptor, "status_code", "int");
			});

		registerGrpcFunction("cef_on_player_connect", "bool",
			[](GrpcFunctionDescriptor& descriptor)
			{
				addGrpcParam(descriptor, "player_id", "int", omp::ext::v1::ENTITY_TYPE_PLAYER);
				addGrpcParam(descriptor, "ip", "char*");
			},
			&CefComponent::grpcOnPlayerConnect);

		registerGrpcFunction("cef_on_player_disconnect", "bool",
			[](GrpcFunctionDescriptor& descriptor)
			{
				addGrpcParam(descriptor, "player_id", "int", omp::ext::v1::ENTITY_TYPE_PLAYER);
			},
			&CefComponent::grpcOnPlayerDisconnect);

		registerGrpcFunction("cef_create_browser", "bool",
			[](GrpcFunctionDescriptor& descriptor)
			{
				addGrpcParam(descriptor, "player_id", "int", omp::ext::v1::ENTITY_TYPE_PLAYER);
				addGrpcParam(descriptor, "browser_id", "int");
				addGrpcParam(descriptor, "url", "char*");
				addGrpcParam(descriptor, "hidden", "bool");
				addGrpcParam(descriptor, "focused", "bool");
			},
			&CefComponent::grpcCreateBrowser);

		registerGrpcFunction("cef_destroy_browser", "bool",
			[](GrpcFunctionDescriptor& descriptor)
			{
				addGrpcParam(descriptor, "player_id", "int", omp::ext::v1::ENTITY_TYPE_PLAYER);
				addGrpcParam(descriptor, "browser_id", "int");
			},
			&CefComponent::grpcDestroyBrowser);

		registerGrpcFunction("cef_emit_event", "bool",
			[](GrpcFunctionDescriptor& descriptor)
			{
				addGrpcParam(descriptor, "player_id", "int", omp::ext::v1::ENTITY_TYPE_PLAYER);
				addGrpcParam(descriptor, "event", "char*");
				addGrpcParam(descriptor, "args", "list");
			},
			&CefComponent::grpcEmitEvent);

		registerGrpcFunction("cef_subscribe", "bool",
			[](GrpcFunctionDescriptor& descriptor)
			{
				addGrpcParam(descriptor, "event", "char*");
			},
			&CefComponent::grpcSubscribe);

		registerGrpcFunction("cef_always_listen_keys", "bool",
			[](GrpcFunctionDescriptor& descriptor)
			{
				addGrpcParam(descriptor, "player_id", "int", omp::ext::v1::ENTITY_TYPE_PLAYER);
				addGrpcParam(descriptor, "browser_id", "int");
				addGrpcParam(descriptor, "listen", "bool");
			},
			&CefComponent::grpcAlwaysListenKeys);

		registerGrpcFunction("cef_hide_browser", "bool",
			[](GrpcFunctionDescriptor& descriptor)
			{
				addGrpcParam(descriptor, "player_id", "int", omp::ext::v1::ENTITY_TYPE_PLAYER);
				addGrpcParam(descriptor, "browser_id", "int");
				addGrpcParam(descriptor, "hide", "bool");
			},
			&CefComponent::grpcHideBrowser);

		registerGrpcFunction("cef_focus_browser", "bool",
			[](GrpcFunctionDescriptor& descriptor)
			{
				addGrpcParam(descriptor, "player_id", "int", omp::ext::v1::ENTITY_TYPE_PLAYER);
				addGrpcParam(descriptor, "browser_id", "int");
				addGrpcParam(descriptor, "focused", "bool");
			},
			&CefComponent::grpcFocusBrowser);

		registerGrpcFunction("cef_player_has_plugin", "bool",
			[](GrpcFunctionDescriptor& descriptor)
			{
				addGrpcParam(descriptor, "player_id", "int", omp::ext::v1::ENTITY_TYPE_PLAYER);
			},
			&CefComponent::grpcPlayerHasPlugin);

		registerGrpcFunction("cef_create_ext_browser", "bool",
			[](GrpcFunctionDescriptor& descriptor)
			{
				addGrpcParam(descriptor, "player_id", "int", omp::ext::v1::ENTITY_TYPE_PLAYER);
				addGrpcParam(descriptor, "browser_id", "int");
				addGrpcParam(descriptor, "texture", "char*");
				addGrpcParam(descriptor, "url", "char*");
				addGrpcParam(descriptor, "scale", "int");
			},
			&CefComponent::grpcCreateExternalBrowser);

		registerGrpcFunction("cef_append_to_object", "bool",
			[](GrpcFunctionDescriptor& descriptor)
			{
				addGrpcParam(descriptor, "player_id", "int", omp::ext::v1::ENTITY_TYPE_PLAYER);
				addGrpcParam(descriptor, "browser_id", "int");
				addGrpcParam(descriptor, "object_id", "int");
			},
			&CefComponent::grpcAppendToObject);

		registerGrpcFunction("cef_remove_from_object", "bool",
			[](GrpcFunctionDescriptor& descriptor)
			{
				addGrpcParam(descriptor, "player_id", "int", omp::ext::v1::ENTITY_TYPE_PLAYER);
				addGrpcParam(descriptor, "browser_id", "int");
				addGrpcParam(descriptor, "object_id", "int");
			},
			&CefComponent::grpcRemoveFromObject);

		registerGrpcFunction("cef_toggle_dev_tools", "bool",
			[](GrpcFunctionDescriptor& descriptor)
			{
				addGrpcParam(descriptor, "player_id", "int", omp::ext::v1::ENTITY_TYPE_PLAYER);
				addGrpcParam(descriptor, "browser_id", "int");
				addGrpcParam(descriptor, "enabled", "bool");
			},
			&CefComponent::grpcToggleDevTools);

		registerGrpcFunction("cef_set_audio_settings", "bool",
			[](GrpcFunctionDescriptor& descriptor)
			{
				addGrpcParam(descriptor, "player_id", "int", omp::ext::v1::ENTITY_TYPE_PLAYER);
				addGrpcParam(descriptor, "browser_id", "int");
				addGrpcParam(descriptor, "max_distance", "float");
				addGrpcParam(descriptor, "reference_distance", "float");
			},
			&CefComponent::grpcSetAudioSettings);

		registerGrpcFunction("cef_load_url", "bool",
			[](GrpcFunctionDescriptor& descriptor)
			{
				addGrpcParam(descriptor, "player_id", "int", omp::ext::v1::ENTITY_TYPE_PLAYER);
				addGrpcParam(descriptor, "browser_id", "int");
				addGrpcParam(descriptor, "url", "char*");
			},
			&CefComponent::grpcLoadUrl);
	}

	void emitGrpcEvent(const std::string& name, std::vector<GrpcNamedValue> args)
	{
		if (!grpc_)
		{
			return;
		}

		Span<const GrpcNamedValue> view(args.data(), args.size());
		grpc_->emitEvent(StringView(name.data(), name.length()), view);
	}

	void emitGrpcBrowserEvent(const std::string& name, int playerID, const std::string& arguments)
	{
		if (!grpc_ || grpcBrowserEvents_.find(name) == grpcBrowserEvents_.end())
		{
			return;
		}

		std::vector<GrpcNamedValue> args;
		args.push_back(grpcNamedPlayer("player_id", playerID));
		args.push_back(grpcNamedString("event", name));
		args.push_back(grpcNamedString("arguments", arguments));
		emitGrpcEvent(grpcBrowserEventName(name), std::move(args));
	}

	static CefComponent* grpcSelf(void* userData, GrpcReply& reply, bool requireServer = true)
	{
		auto* self = static_cast<CefComponent*>(userData);
		if (!self || (requireServer && !self->server_))
		{
			grpcFail(reply, "CEF_NOT_READY", "CEF server is not ready");
			return nullptr;
		}
		return self;
	}

	static bool grpcPushEventArgument(EventArguments& arguments, const omp::ext::v1::Value& value, GrpcReply& reply)
	{
		if (value.has_string_value())
		{
			arguments.push_string(asRustStr(value.string_value()));
			return true;
		}
		if (value.has_float_value())
		{
			arguments.push_float(value.float_value());
			return true;
		}
		if (value.has_double_value())
		{
			arguments.push_float(static_cast<float>(value.double_value()));
			return true;
		}
		if (value.has_bool_value())
		{
			arguments.push_integer(value.bool_value() ? 1 : 0);
			return true;
		}

		int integer = 0;
		if (grpcReadIntValue(value, integer))
		{
			arguments.push_integer(integer);
			return true;
		}

		grpcFail(reply, "BAD_ARGUMENT", "CEF event arguments must be string, integer, float, or bool");
		return false;
	}

	static void grpcOnPlayerConnect(const GrpcRequest& request, GrpcReply& reply, void* userData)
	{
		CefComponent* self = grpcSelf(userData, reply);
		if (!self)
		{
			return;
		}

		int playerID = 0;
		std::string ip;
		if (!grpcReadRequired(request, reply, "player_id", 0, grpcReadPlayerValue, playerID, "player")
			|| !grpcReadRequired(request, reply, "ip", 1, grpcReadStringValue, ip, "string"))
		{
			return;
		}

		grpcReturnBool(reply, (*self->server_)->allow_connection(playerID, asRustStr(ip)));
	}

	static void grpcOnPlayerDisconnect(const GrpcRequest& request, GrpcReply& reply, void* userData)
	{
		CefComponent* self = grpcSelf(userData, reply);
		if (!self)
		{
			return;
		}

		int playerID = 0;
		if (!grpcReadRequired(request, reply, "player_id", 0, grpcReadPlayerValue, playerID, "player"))
		{
			return;
		}

		(*self->server_)->remove_connection(playerID);
		grpcReturnBool(reply, true);
	}

	static void grpcCreateBrowser(const GrpcRequest& request, GrpcReply& reply, void* userData)
	{
		CefComponent* self = grpcSelf(userData, reply);
		if (!self)
		{
			return;
		}

		int playerID = 0;
		int browserID = 0;
		std::string url;
		bool hidden = false;
		bool focused = true;
		if (!grpcReadRequired(request, reply, "player_id", 0, grpcReadPlayerValue, playerID, "player")
			|| !grpcReadRequired(request, reply, "browser_id", 1, grpcReadIntValue, browserID, "int")
			|| !grpcReadRequired(request, reply, "url", 2, grpcReadStringValue, url, "string")
			|| !grpcReadRequired(request, reply, "hidden", 3, grpcReadBoolValue, hidden, "bool")
			|| !grpcReadRequired(request, reply, "focused", 4, grpcReadBoolValue, focused, "bool"))
		{
			return;
		}

		(*self->server_)->create_browser(playerID, browserID, asRustStr(url), hidden, focused);
		grpcReturnBool(reply, true);
	}

	static void grpcDestroyBrowser(const GrpcRequest& request, GrpcReply& reply, void* userData)
	{
		CefComponent* self = grpcSelf(userData, reply);
		if (!self)
		{
			return;
		}

		int playerID = 0;
		int browserID = 0;
		if (!grpcReadRequired(request, reply, "player_id", 0, grpcReadPlayerValue, playerID, "player")
			|| !grpcReadRequired(request, reply, "browser_id", 1, grpcReadIntValue, browserID, "int"))
		{
			return;
		}

		(*self->server_)->destroy_browser(playerID, browserID);
		grpcReturnBool(reply, true);
	}

	static void grpcEmitEvent(const GrpcRequest& request, GrpcReply& reply, void* userData)
	{
		CefComponent* self = grpcSelf(userData, reply);
		if (!self)
		{
			return;
		}

		int playerID = 0;
		std::string eventName;
		if (!grpcReadRequired(request, reply, "player_id", 0, grpcReadPlayerValue, playerID, "player")
			|| !grpcReadRequired(request, reply, "event", 1, grpcReadStringValue, eventName, "string"))
		{
			return;
		}

		auto arguments = samp_cef::openmp::new_event_arguments();
		if (const auto* args = grpcArg(request, "args", -1))
		{
			if (!args->has_list_value())
			{
				grpcFail(reply, "BAD_ARGUMENT", "Expected list argument `args`");
				return;
			}

			for (const auto& value : args->list_value().values())
			{
				if (!grpcPushEventArgument(*arguments, value, reply))
				{
					return;
				}
			}
		}
		else
		{
			for (int index = 0; index < request.args_size(); ++index)
			{
				const auto& arg = request.args(index);
				if (index < 2 || arg.name() == "player_id" || arg.name() == "event")
				{
					continue;
				}
				if (!grpcPushEventArgument(*arguments, arg.value(), reply))
				{
					return;
				}
			}
		}

		(*self->server_)->emit_event(playerID, asRustStr(eventName), *arguments);
		grpcReturnBool(reply, true);
	}

	static void grpcSubscribe(const GrpcRequest& request, GrpcReply& reply, void* userData)
	{
		CefComponent* self = grpcSelf(userData, reply, false);
		if (!self)
		{
			return;
		}

		std::string eventName;
		if (!grpcReadRequired(request, reply, "event", 0, grpcReadStringValue, eventName, "string"))
		{
			return;
		}

		const bool registered = self->registerGrpcBrowserEvent(eventName);
		if (!registered && self->core_)
		{
			self->core_->logLn(LogLevel::Warning, "[CEF] Failed to register gRPC event `%s`", eventName.c_str());
		}
		grpcReturnBool(reply, registered);
	}

	static void grpcAlwaysListenKeys(const GrpcRequest& request, GrpcReply& reply, void* userData)
	{
		CefComponent* self = grpcSelf(userData, reply);
		if (!self)
		{
			return;
		}

		int playerID = 0;
		int browserID = 0;
		bool listen = false;
		if (!grpcReadRequired(request, reply, "player_id", 0, grpcReadPlayerValue, playerID, "player")
			|| !grpcReadRequired(request, reply, "browser_id", 1, grpcReadIntValue, browserID, "int")
			|| !grpcReadRequired(request, reply, "listen", 2, grpcReadBoolValue, listen, "bool"))
		{
			return;
		}

		(*self->server_)->always_listen_keys(playerID, browserID, listen);
		grpcReturnBool(reply, true);
	}

	static void grpcHideBrowser(const GrpcRequest& request, GrpcReply& reply, void* userData)
	{
		CefComponent* self = grpcSelf(userData, reply);
		if (!self)
		{
			return;
		}

		int playerID = 0;
		int browserID = 0;
		bool hide = false;
		if (!grpcReadRequired(request, reply, "player_id", 0, grpcReadPlayerValue, playerID, "player")
			|| !grpcReadRequired(request, reply, "browser_id", 1, grpcReadIntValue, browserID, "int")
			|| !grpcReadRequired(request, reply, "hide", 2, grpcReadBoolValue, hide, "bool"))
		{
			return;
		}

		(*self->server_)->hide_browser(playerID, browserID, hide);
		grpcReturnBool(reply, true);
	}

	static void grpcFocusBrowser(const GrpcRequest& request, GrpcReply& reply, void* userData)
	{
		CefComponent* self = grpcSelf(userData, reply);
		if (!self)
		{
			return;
		}

		int playerID = 0;
		int browserID = 0;
		bool focused = false;
		if (!grpcReadRequired(request, reply, "player_id", 0, grpcReadPlayerValue, playerID, "player")
			|| !grpcReadRequired(request, reply, "browser_id", 1, grpcReadIntValue, browserID, "int")
			|| !grpcReadRequired(request, reply, "focused", 2, grpcReadBoolValue, focused, "bool"))
		{
			return;
		}

		(*self->server_)->focus_browser(playerID, browserID, focused);
		grpcReturnBool(reply, true);
	}

	static void grpcPlayerHasPlugin(const GrpcRequest& request, GrpcReply& reply, void* userData)
	{
		CefComponent* self = grpcSelf(userData, reply);
		if (!self)
		{
			return;
		}

		int playerID = 0;
		if (!grpcReadRequired(request, reply, "player_id", 0, grpcReadPlayerValue, playerID, "player"))
		{
			return;
		}

		grpcReturnBool(reply, (*self->server_)->has_plugin(playerID));
	}

	static void grpcCreateExternalBrowser(const GrpcRequest& request, GrpcReply& reply, void* userData)
	{
		CefComponent* self = grpcSelf(userData, reply);
		if (!self)
		{
			return;
		}

		int playerID = 0;
		int browserID = 0;
		int scale = 0;
		std::string texture;
		std::string url;
		if (!grpcReadRequired(request, reply, "player_id", 0, grpcReadPlayerValue, playerID, "player")
			|| !grpcReadRequired(request, reply, "browser_id", 1, grpcReadIntValue, browserID, "int")
			|| !grpcReadRequired(request, reply, "texture", 2, grpcReadStringValue, texture, "string")
			|| !grpcReadRequired(request, reply, "url", 3, grpcReadStringValue, url, "string")
			|| !grpcReadRequired(request, reply, "scale", 4, grpcReadIntValue, scale, "int"))
		{
			return;
		}

		(*self->server_)->create_external_browser(playerID, browserID, asRustStr(texture), asRustStr(url), scale);
		grpcReturnBool(reply, true);
	}

	static void grpcAppendToObject(const GrpcRequest& request, GrpcReply& reply, void* userData)
	{
		CefComponent* self = grpcSelf(userData, reply);
		if (!self)
		{
			return;
		}

		int playerID = 0;
		int browserID = 0;
		int objectID = 0;
		if (!grpcReadRequired(request, reply, "player_id", 0, grpcReadPlayerValue, playerID, "player")
			|| !grpcReadRequired(request, reply, "browser_id", 1, grpcReadIntValue, browserID, "int")
			|| !grpcReadRequired(request, reply, "object_id", 2, grpcReadIntValue, objectID, "int"))
		{
			return;
		}

		(*self->server_)->append_to_object(playerID, browserID, objectID);
		grpcReturnBool(reply, true);
	}

	static void grpcRemoveFromObject(const GrpcRequest& request, GrpcReply& reply, void* userData)
	{
		CefComponent* self = grpcSelf(userData, reply);
		if (!self)
		{
			return;
		}

		int playerID = 0;
		int browserID = 0;
		int objectID = 0;
		if (!grpcReadRequired(request, reply, "player_id", 0, grpcReadPlayerValue, playerID, "player")
			|| !grpcReadRequired(request, reply, "browser_id", 1, grpcReadIntValue, browserID, "int")
			|| !grpcReadRequired(request, reply, "object_id", 2, grpcReadIntValue, objectID, "int"))
		{
			return;
		}

		(*self->server_)->remove_from_object(playerID, browserID, objectID);
		grpcReturnBool(reply, true);
	}

	static void grpcToggleDevTools(const GrpcRequest& request, GrpcReply& reply, void* userData)
	{
		CefComponent* self = grpcSelf(userData, reply);
		if (!self)
		{
			return;
		}

		int playerID = 0;
		int browserID = 0;
		bool enabled = false;
		if (!grpcReadRequired(request, reply, "player_id", 0, grpcReadPlayerValue, playerID, "player")
			|| !grpcReadRequired(request, reply, "browser_id", 1, grpcReadIntValue, browserID, "int")
			|| !grpcReadRequired(request, reply, "enabled", 2, grpcReadBoolValue, enabled, "bool"))
		{
			return;
		}

		(*self->server_)->toggle_dev_tools(playerID, browserID, enabled);
		grpcReturnBool(reply, true);
	}

	static void grpcSetAudioSettings(const GrpcRequest& request, GrpcReply& reply, void* userData)
	{
		CefComponent* self = grpcSelf(userData, reply);
		if (!self)
		{
			return;
		}

		int playerID = 0;
		std::uint32_t browserID = 0;
		float maxDistance = CEF_DEFAULT_MAX_DIST;
		float referenceDistance = CEF_DEFAULT_REF_DIST;
		if (!grpcReadRequired(request, reply, "player_id", 0, grpcReadPlayerValue, playerID, "player")
			|| !grpcReadRequired(request, reply, "browser_id", 1, grpcReadUInt32Value, browserID, "int")
			|| !grpcReadOptional(request, "max_distance", 2, grpcReadFloatValue, maxDistance)
			|| !grpcReadOptional(request, "reference_distance", 3, grpcReadFloatValue, referenceDistance))
		{
			grpcFail(reply, "BAD_ARGUMENT", "Expected numeric audio settings");
			return;
		}

		(*self->server_)->set_audio_settings(playerID, browserID, maxDistance, referenceDistance);
		grpcReturnBool(reply, true);
	}

	static void grpcLoadUrl(const GrpcRequest& request, GrpcReply& reply, void* userData)
	{
		CefComponent* self = grpcSelf(userData, reply);
		if (!self)
		{
			return;
		}

		int playerID = 0;
		std::uint32_t browserID = 0;
		std::string url;
		if (!grpcReadRequired(request, reply, "player_id", 0, grpcReadPlayerValue, playerID, "player")
			|| !grpcReadRequired(request, reply, "browser_id", 1, grpcReadUInt32Value, browserID, "int")
			|| !grpcReadRequired(request, reply, "url", 2, grpcReadStringValue, url, "string"))
		{
			return;
		}

		(*self->server_)->load_url(playerID, browserID, asRustStr(url));
		grpcReturnBool(reply, true);
	}
#endif

	static CefComponent* component()
	{
		return instance_;
	}

	static cell AMX_NATIVE_CALL nativeOnPlayerConnect(AMX* amx, const cell* params)
	{
		CefComponent* self = component();
		if (!self || !self->server_ || !self->requireParams(params, 2, "cef_on_player_connect"))
		{
			return 0;
		}

		IPawnScript* script = self->scriptFromAmx(amx);
		if (!script)
		{
			return 0;
		}

		std::string ip;
		if (!self->getAmxString(*script, params[2], ip))
		{
			return 0;
		}

		return (*self->server_)->allow_connection(static_cast<int>(params[1]), asRustStr(ip)) ? 1 : 0;
	}

	static cell AMX_NATIVE_CALL nativeOnPlayerDisconnect(AMX*, const cell* params)
	{
		CefComponent* self = component();
		if (!self || !self->server_ || !self->requireParams(params, 1, "cef_on_player_disconnect"))
		{
			return 0;
		}

		(*self->server_)->remove_connection(static_cast<int>(params[1]));
		return 1;
	}

	static cell AMX_NATIVE_CALL nativeCreateBrowser(AMX* amx, const cell* params)
	{
		CefComponent* self = component();
		if (!self || !self->server_ || !self->requireParams(params, 5, "cef_create_browser"))
		{
			return 0;
		}

		IPawnScript* script = self->scriptFromAmx(amx);
		if (!script)
		{
			return 0;
		}

		std::string url;
		if (!self->getAmxString(*script, params[3], url))
		{
			return 0;
		}

		(*self->server_)
			->create_browser(
				static_cast<int>(params[1]),
				static_cast<int>(params[2]),
				asRustStr(url),
				params[4] != 0,
				params[5] != 0);
		return 1;
	}

	static cell AMX_NATIVE_CALL nativeDestroyBrowser(AMX*, const cell* params)
	{
		CefComponent* self = component();
		if (!self || !self->server_ || !self->requireParams(params, 2, "cef_destroy_browser"))
		{
			return 0;
		}

		(*self->server_)->destroy_browser(static_cast<int>(params[1]), static_cast<int>(params[2]));
		return 1;
	}

	static cell AMX_NATIVE_CALL nativeEmitEvent(AMX* amx, const cell* params)
	{
		CefComponent* self = component();
		if (!self || !self->server_ || !self->requireParams(params, 2, "cef_emit_event"))
		{
			return 0;
		}

		const int count = pawnParamCount(params);
		if ((count - 2) % 2 != 0)
		{
			if (self->core_)
			{
				self->core_->logLn(LogLevel::Error, "[CEF] cef_emit_event has an incomplete argument pair");
			}
			return 0;
		}

		IPawnScript* script = self->scriptFromAmx(amx);
		if (!script)
		{
			if (self->core_)
			{
				self->core_->logLn(LogLevel::Error, "[CEF] cef_emit_event could not resolve the Pawn script");
			}
			return 0;
		}

		std::string eventName;
		if (!self->getAmxString(*script, params[2], eventName))
		{
			if (self->core_)
			{
				self->core_->logLn(LogLevel::Error, "[CEF] cef_emit_event could not read the event name");
			}
			return 0;
		}

		auto arguments = samp_cef::openmp::new_event_arguments();
		for (int idx = 3; idx <= count; idx += 2)
		{
			cell typeValue = 0;
			if (!self->getAmxCell(*script, params[idx], typeValue))
			{
				if (self->core_)
				{
					self->core_->logLn(LogLevel::Error, "[CEF] cef_emit_event could not read argument type at index %d", idx);
				}
				return 0;
			}

			const int type = static_cast<int>(typeValue);
			const cell value = params[idx + 1];

			switch (type)
			{
			case 0:
			{
				std::string stringValue;
				if (!self->getAmxString(*script, value, stringValue))
				{
					if (self->core_)
					{
						self->core_->logLn(LogLevel::Error, "[CEF] cef_emit_event could not read string argument at index %d", idx);
					}
					return 0;
				}
				arguments->push_string(asRustStr(stringValue));
				break;
			}

			case 1:
			{
				cell integerValue = 0;
				if (!self->getAmxCell(*script, value, integerValue))
				{
					return 0;
				}
				arguments->push_integer(static_cast<int>(integerValue));
				break;
			}

			case 2:
			{
				cell floatValue = 0;
				if (!self->getAmxCell(*script, value, floatValue))
				{
					return 0;
				}
				arguments->push_float(amx_ctof(floatValue));
				break;
			}

			default:
				if (self->core_)
				{
					self->core_->logLn(LogLevel::Error, "[CEF] cef_emit_event has invalid argument type %d at index %d", type, idx);
				}
				return 0;
			}
		}

		(*self->server_)->emit_event(static_cast<int>(params[1]), asRustStr(eventName), *arguments);
		return 1;
	}

	static cell AMX_NATIVE_CALL nativeSubscribe(AMX* amx, const cell* params)
	{
		CefComponent* self = component();
		if (!self || !self->requireParams(params, 2, "cef_subscribe"))
		{
			return 0;
		}

		IPawnScript* script = self->scriptFromAmx(amx);
		if (!script)
		{
			return 0;
		}

		std::string eventName;
		std::string callback;
		if (!self->getAmxString(*script, params[1], eventName) || !self->getAmxString(*script, params[2], callback))
		{
			return 0;
		}

		self->subscriptions_[eventName] = Subscription { script, callback };
		return 1;
	}

	static cell AMX_NATIVE_CALL nativeAlwaysListenKeys(AMX*, const cell* params)
	{
		CefComponent* self = component();
		if (!self || !self->server_ || !self->requireParams(params, 3, "cef_always_listen_keys"))
		{
			return 0;
		}

		(*self->server_)
			->always_listen_keys(static_cast<int>(params[1]), static_cast<int>(params[2]), params[3] != 0);
		return 1;
	}

	static cell AMX_NATIVE_CALL nativeHideBrowser(AMX*, const cell* params)
	{
		CefComponent* self = component();
		if (!self || !self->server_ || !self->requireParams(params, 3, "cef_hide_browser"))
		{
			return 0;
		}

		(*self->server_)->hide_browser(static_cast<int>(params[1]), static_cast<int>(params[2]), params[3] != 0);
		return 1;
	}

	static cell AMX_NATIVE_CALL nativeFocusBrowser(AMX*, const cell* params)
	{
		CefComponent* self = component();
		if (!self || !self->server_ || !self->requireParams(params, 3, "cef_focus_browser"))
		{
			return 0;
		}

		(*self->server_)->focus_browser(static_cast<int>(params[1]), static_cast<int>(params[2]), params[3] != 0);
		return 1;
	}

	static cell AMX_NATIVE_CALL nativePlayerHasPlugin(AMX*, const cell* params)
	{
		CefComponent* self = component();
		if (!self || !self->server_ || !self->requireParams(params, 1, "cef_player_has_plugin"))
		{
			return 0;
		}

		return (*self->server_)->has_plugin(static_cast<int>(params[1])) ? 1 : 0;
	}

	static cell AMX_NATIVE_CALL nativeCreateExternalBrowser(AMX* amx, const cell* params)
	{
		CefComponent* self = component();
		if (!self || !self->server_ || !self->requireParams(params, 5, "cef_create_ext_browser"))
		{
			return 0;
		}

		IPawnScript* script = self->scriptFromAmx(amx);
		if (!script)
		{
			return 0;
		}

		std::string texture;
		std::string url;
		if (!self->getAmxString(*script, params[3], texture) || !self->getAmxString(*script, params[4], url))
		{
			return 0;
		}

		(*self->server_)
			->create_external_browser(
				static_cast<int>(params[1]),
				static_cast<int>(params[2]),
				asRustStr(texture),
				asRustStr(url),
				static_cast<int>(params[5]));
		return 1;
	}

	static cell AMX_NATIVE_CALL nativeAppendToObject(AMX*, const cell* params)
	{
		CefComponent* self = component();
		if (!self || !self->server_ || !self->requireParams(params, 3, "cef_append_to_object"))
		{
			return 0;
		}

		(*self->server_)
			->append_to_object(static_cast<int>(params[1]), static_cast<int>(params[2]), static_cast<int>(params[3]));
		return 1;
	}

	static cell AMX_NATIVE_CALL nativeRemoveFromObject(AMX*, const cell* params)
	{
		CefComponent* self = component();
		if (!self || !self->server_ || !self->requireParams(params, 3, "cef_remove_from_object"))
		{
			return 0;
		}

		(*self->server_)
			->remove_from_object(static_cast<int>(params[1]), static_cast<int>(params[2]), static_cast<int>(params[3]));
		return 1;
	}

	static cell AMX_NATIVE_CALL nativeToggleDevTools(AMX*, const cell* params)
	{
		CefComponent* self = component();
		if (!self || !self->server_ || !self->requireParams(params, 3, "cef_toggle_dev_tools"))
		{
			return 0;
		}

		(*self->server_)->toggle_dev_tools(static_cast<int>(params[1]), static_cast<int>(params[2]), params[3] != 0);
		return 1;
	}

	static cell AMX_NATIVE_CALL nativeSetAudioSettings(AMX*, const cell* params)
	{
		CefComponent* self = component();
		if (!self || !self->server_ || !self->requireParams(params, 4, "cef_set_audio_settings"))
		{
			return 0;
		}

		(*self->server_)
			->set_audio_settings(
				static_cast<int>(params[1]),
				static_cast<std::uint32_t>(params[2]),
				amx_ctof(params[3]),
				amx_ctof(params[4]));
		return 1;
	}

	static cell AMX_NATIVE_CALL nativeLoadUrl(AMX* amx, const cell* params)
	{
		CefComponent* self = component();
		if (!self || !self->server_ || !self->requireParams(params, 3, "cef_load_url"))
		{
			return 0;
		}

		IPawnScript* script = self->scriptFromAmx(amx);
		if (!script)
		{
			return 0;
		}

		std::string url;
		if (!self->getAmxString(*script, params[3], url))
		{
			return 0;
		}

		(*self->server_)->load_url(static_cast<int>(params[1]), static_cast<std::uint32_t>(params[2]), asRustStr(url));
		return 1;
	}
};

CefComponent* CefComponent::instance_ = nullptr;
}

COMPONENT_ENTRY_POINT()
{
	return new CefComponent();
}
