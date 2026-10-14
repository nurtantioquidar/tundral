<?php

namespace Shop\Tests;

use PHPUnit\Framework\TestCase;
use Shop\Models\User;
use Shop\Service\UserService;

class UserServiceTest extends TestCase
{
    public function testParse(): void
    {
        $user = User::parse(' ada ');
        $this->assertSame('ada', $user->display());
    }

    public function testRegister(): void
    {
        $service = UserService::make($this->createMock(\Shop\Models\Repository::class));
        $this->assertInstanceOf(User::class, $service->register('bob'));
    }
}
