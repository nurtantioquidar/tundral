<?php

namespace Shop\Service;

use Shop\Models\{User, Repository};
use Shop\Models\Status as UserStatus;
use Psr\Log\LoggerInterface;
use InvalidArgumentException;

class UserService
{
    private array $cache = [];

    public function __construct(
        private Repository $repo,
        private ?LoggerInterface $logger = null,
    ) {
    }

    public function register(string $name): User
    {
        if ($name === '') {
            throw new InvalidArgumentException('empty name');
        }
        $user = new User($name);
        $user->save();
        $this->repo->add($user);
        $this->log("registered {$name}");
        return $user;
    }

    public function ban(User $user): void
    {
        $user->status = UserStatus::Banned;
        $this->log('banned');
    }

    public static function make(Repository $repo): static
    {
        return new self($repo);
    }

    private function log(string $message): void
    {
        $this->logger?->info($message);
        helper($message);
    }
}

function helper(string $value): string
{
    return strtolower($value);
}
