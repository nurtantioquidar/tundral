<html><body>
<?php

require_once __DIR__ . '/../src/helpers.php';
require __DIR__ . '/../src/Models/Model.php';
include 'src/Models/User.php';

use Shop\Models\User;
use Shop\Service\UserService;

function main(array $argv): int
{
    $service = UserService::make(new \Shop\Repo\MemoryRepo());
    $user = $service->register($argv[1] ?? 'guest');
    echo $user->display();
    \Shop\slugify('x');
    unknown_function();
    return 0;
}

exit(main($argv));
?>
</body></html>
