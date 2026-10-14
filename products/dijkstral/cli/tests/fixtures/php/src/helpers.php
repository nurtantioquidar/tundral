<?php

namespace Shop;

const DEFAULT_ROLE = 'member';

/**
 * Turn any display name into a url-safe slug.
 */
function slugify(string $value): string
{
    return strtolower(trim(preg_replace('/[^a-z0-9]+/i', '-', $value), '-'));
}

function helper(string $value): string
{
    // TODO: share with the service helper.
    return slugify($value);
}
