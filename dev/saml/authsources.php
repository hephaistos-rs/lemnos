<?php
// Test users for the SAML IdP in compose.dev.yml. Development only.
// Each key is "username:password".
$config = [
    'admin' => ['core:AdminPassword'],
    'example-userpass' => [
        'exampleauth:UserPass',
        'alice:alice-password' => [
            'uid' => ['alice'],
            'email' => ['alice@lemnos.test'],
            'displayName' => ['Alice Liddell'],
        ],
        'bob:bob-password' => [
            'uid' => ['bob'],
            'email' => ['bob@lemnos.test'],
            'displayName' => ['Bob Builder'],
        ],
    ],
];
