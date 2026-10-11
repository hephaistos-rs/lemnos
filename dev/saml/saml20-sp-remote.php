<?php
// Lemnos, as the service provider the SAML IdP in compose.dev.yml talks to.
$metadata['http://localhost:3000/auth/sso/saml'] = [
    'AssertionConsumerService' => 'http://localhost:3000/auth/sso/saml/acs',
    'NameIDFormat' => 'urn:oasis:names:tc:SAML:2.0:nameid-format:persistent',
];
