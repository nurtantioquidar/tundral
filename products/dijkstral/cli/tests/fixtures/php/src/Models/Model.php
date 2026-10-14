<?php

declare(strict_types=1);

namespace Shop\Models;

/**
 * Base model shared by every persisted entity in the shop.
 */
abstract class Model implements \JsonSerializable
{
    protected int $id = 0;

    abstract public function table(): string;

    public function save(): bool
    {
        // NOTE: persistence is faked in this fixture.
        return $this->validate();
    }

    protected function validate(): bool
    {
        return true;
    }

    public function jsonSerialize(): mixed
    {
        return ['id' => $this->id];
    }
}
