//! Conversions between Lua values and JSON.

use mlua::serde::SerializeOptions;
use mlua::{Lua, LuaSerdeExt};

/// JSON to Lua. JSON null becomes nil, so `if video.channel then` works.
pub fn json_to_lua(lua: &Lua, value: &serde_json::Value) -> mlua::Result<mlua::Value> {
    lua.to_value_with(
        value,
        SerializeOptions::new()
            .serialize_none_to_null(false)
            .serialize_unit_to_null(false),
    )
}

/// Lua to JSON. Userdata (such as widgets) can't be represented and becomes
/// null.
pub fn lua_to_json(lua: &Lua, value: mlua::Value) -> mlua::Result<serde_json::Value> {
    match value {
        mlua::Value::UserData(_) | mlua::Value::Function(_) | mlua::Value::Thread(_) => {
            Ok(serde_json::Value::Null)
        }
        other => lua.from_value(other),
    }
}
