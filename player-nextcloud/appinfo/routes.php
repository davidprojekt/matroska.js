<?php

declare(strict_types=1);

// Admin-only endpoint that saves + validates the license key (see LicenseController). Loaded
// automatically by Nextcloud; no manual registration in Application.php is needed.
return [
	'routes' => [
		['name' => 'license#save', 'url' => '/settings/license', 'verb' => 'POST'],
	],
];
