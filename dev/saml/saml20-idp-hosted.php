<?php
// The SAML IdP in compose.dev.yml. Development only.
$metadata['__DYNAMIC:1__'] = [
    'host' => '__DEFAULT__',
    'privatekey' => 'server.pem',
    'certificate' => 'server.crt',
    'auth' => 'example-userpass',

    // Lemnos only accepts SHA-256 or stronger.
    'signature.algorithm' => 'http://www.w3.org/2001/04/xmldsig-more#rsa-sha256',

    'authproc' => [
        // Send the user's `uid` as a persistent NameID. The default is a
        // transient one that changes on every sign-in, which Lemnos refuses
        // because it can't identify an account.
        3 => [
            'class' => 'saml:AttributeNameID',
            'attribute' => 'uid',
            'Format' => 'urn:oasis:names:tc:SAML:2.0:nameid-format:persistent',
        ],
    ],
];
