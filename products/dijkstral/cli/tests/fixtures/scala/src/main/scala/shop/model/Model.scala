package shop.model

import java.time.Instant

/** Anything with a stable identifier. */
trait Entity[K] {
  def id: K

  def describe: String = s"entity $id"
}

/**
 * Base class for persisted models.
 * NOTE: timestamps are in UTC.
 */
abstract class Model(val id: String) extends Entity[String] {
  val createdAt: Instant = Instant.now()

  def validate(): Boolean

  def touch(): Model = {
    validate()
    this
  }
}

sealed trait Status

case object Active extends Status

case object Inactive extends Status

enum Color {
  case Red, Green

  def label: String = toString.toLowerCase
}

type UserId = String
