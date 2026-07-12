<?php

declare(strict_types=1);

namespace OCA\MkvPlayer\Listener;

use OCA\MkvPlayer\AppInfo\Application;
use OCA\MkvPlayer\Service\ConfigService;
use OCP\AppFramework\Services\IInitialState;
use OCP\EventDispatcher\Event;
use OCP\EventDispatcher\IEventListener;
use OCP\Util;

/**
 * Loads the Viewer-handler script + styles and seeds the ffmpeg config as initial state. Bound to
 * every event that marks a context where the Viewer can be opened (authenticated Files, explicit
 * LoadViewer, and public link shares) — see Application::register(). addScript/addStyle dedupe per
 * request, so binding to several events that may co-fire is harmless.
 *
 * @template-implements IEventListener<Event>
 */
class LoadViewerListener implements IEventListener {
	public function __construct(
		private IInitialState $initialState,
		private ConfigService $config,
	) {
	}

	public function handle(Event $event): void {
		// Load our entry after the Viewer's own scripts so window.OCA.Viewer exists when we
		// register our handler. Util::addScript resolves `matroskaplayer-main` to js/matroskaplayer-main.mjs.
		Util::addScript(Application::APP_ID, 'matroskaplayer-main', 'viewer');
		Util::addStyle(Application::APP_ID, 'matroskaplayer-main');

		$this->initialState->provideInitialState('config', $this->config->getFrontendConfig());
	}
}
