<?php

namespace Shop\Models;

enum Status: string
{
    case Active = 'active';
    case Banned = 'banned';

    public function label(): string
    {
        return ucfirst($this->value);
    }
}
