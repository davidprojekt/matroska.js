<?php

declare(strict_types=1);

namespace OCA\MkvPlayer\AppInfo;

use OCA\Files\Event\LoadAdditionalScriptsEvent;
use OCA\Files_Sharing\Event\BeforeTemplateRenderedEvent as ShareBeforeTemplateRenderedEvent;
use OCA\MkvPlayer\Listener\CspListener;
use OCA\MkvPlayer\Listener\LoadViewerListener;
use OCA\MkvPlayer\Settings\DeclarativeAdminSettings;
use OCA\Viewer\Event\LoadViewer;
use OCP\AppFramework\App;
use OCP\AppFramework\Bootstrap\IBootContext;
use OCP\AppFramework\Bootstrap\IBootstrap;
use OCP\AppFramework\Bootstrap\IRegistrationContext;
use OCP\Security\CSP\AddContentSecurityPolicyEvent;

class Application extends App implements IBootstrap {
	public const APP_ID = 'matroskaplayer';

	public function __construct() {
		parent::__construct(self::APP_ID);
	}

	public function register(IRegistrationContext $context): void {
		// Inject the Viewer handler script wherever the Viewer can be opened:
		//  - LoadViewer: apps that explicitly load the Viewer.
		//  - LoadAdditionalScriptsEvent: the authenticated Files app UI.
		//  - Files_Sharing BeforeTemplateRenderedEvent: public link share pages.
		$context->registerEventListener(LoadViewer::class, LoadViewerListener::class);
		$context->registerEventListener(LoadAdditionalScriptsEvent::class, LoadViewerListener::class);
		$context->registerEventListener(ShareBeforeTemplateRenderedEvent::class, LoadViewerListener::class);

		// Relax the CSP so the player's WASM + module workers (mkv-player, jassub, ffmpeg) run.
		$context->registerEventListener(AddContentSecurityPolicyEvent::class, CspListener::class);

		// Admin settings for the optional ffmpeg.wasm transcoder (the section is declared in
		// info.xml). Nextcloud renders the form and persists values to appconfig.
		$context->registerDeclarativeSettings(DeclarativeAdminSettings::class);
	}

	public function boot(IBootContext $context): void {
	}
}
