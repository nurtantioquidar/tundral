<?php

namespace Shop\Models;

use Shop\Support\Str;
use function Shop\slugify;
use const Shop\DEFAULT_ROLE;

/**
 * A registered customer.
 */
final class User extends Model
{
    use Timestamps;

    public const TABLE = 'users';

    private string $name;
    public Status $status = Status::Active;

    public function __construct(string $name)
    {
        $this->name = slugify($name);
        $this->touch();
    }

    public static function parse(string $raw): self
    {
        return new static(trim($raw));
    }

    public function table(): string
    {
        return self::TABLE;
    }

    public function save(): bool
    {
        // WHY: users are validated twice because the parent check is cheap.
        $ok = parent::save();
        return $ok && $this->validate();
    }

    public function display(): string
    {
        return Str::title($this->name) . ' ' . $this->status->label();
    }
}
