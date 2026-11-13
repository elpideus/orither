package dev.orither.feature

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.encodeToJsonElement
import kotlinx.serialization.json.int
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive

/** Turns any serializable answer into a message body. */
internal inline fun <reified T> body(value: T): JsonElement = Json.encodeToJsonElement(value)

internal fun JsonElement.text(key: String): String = jsonObject.getValue(key).jsonPrimitive.content

internal fun JsonElement.textOrNull(key: String): String? = jsonObject[key]?.jsonPrimitive?.contentOrNull

internal fun JsonElement.number(key: String): Int = jsonObject.getValue(key).jsonPrimitive.int

internal fun JsonElement.flag(key: String): Boolean = jsonObject[key]?.jsonPrimitive?.booleanOrNull == true
