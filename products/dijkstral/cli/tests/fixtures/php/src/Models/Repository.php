<?php

namespace Shop\Models;

interface Repository
{
    public function find(int $id): ?Model;

    public function add(Model $model): void;
}

trait Timestamps
{
    public ?string $createdAt = null;

    public function touch(): void
    {
        $this->createdAt = date('c');
    }
}
