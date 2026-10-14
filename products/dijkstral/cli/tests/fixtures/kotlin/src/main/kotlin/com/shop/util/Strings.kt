package com.shop.util

/** Lowercase, dash-separated form of [s]. */
fun slugify(s: String): String = s.trim().lowercase().replace(' ', '-')

object Formatter {
    fun shout(s: String): String = s.uppercase()

    fun whisper(s: String) = slugify(s)
}

fun String.words(): List<String> = split(' ')

internal fun <T> List<T>.second(): T = this[1]
