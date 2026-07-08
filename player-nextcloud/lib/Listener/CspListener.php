<?php

declare(strict_types=1);

namespace OCA\MkvPlayer\Listener;

use OCA\MkvPlayer\Service\ConfigService;
use OCP\AppFramework\Http\EmptyContentSecurityPolicy;
use OCP\EventDispatcher\Event;
use OCP\EventDispatcher\IEventListener;
use OCP\Security\CSP\AddContentSecurityPolicyEvent;

/**
 * Relaxes Nextcloud's default CSP so the player's WebAssembly + module workers can run:
 *  - WASM compilation (mkv-player remuxer, jassub/libass, ffmpeg) needs 'wasm-unsafe-eval'.
 *  - jassub and ffmpeg spawn workers, and ffmpeg loads its core via a blob: worker (toBlobURL),
 *    so worker-src/child-src must allow blob: (and 'self' for our own emitted worker assets).
 *  - MSE plays from a blob: MediaSource URL and subtitles/fonts from blob: URLs → media-src blob:.
 *  - When an admin points ffmpeg at an external CDN, that origin must be reachable (connect-src)
 *    to fetch the core/wasm.
 *
 * This is added via AddContentSecurityPolicyEvent, so it merges into the CSP of every response.
 *
 * @template-implements IEventListener<Event>
 */
class CspListener implements IEventListener {
	public function __construct(
		private ConfigService $config,
	) {
	}

	public function handle(Event $event): void {
		if (!($event instanceof AddContentSecurityPolicyEvent)) {
			return;
		}

		$policy = new EmptyContentSecurityPolicy();
		$policy->allowEvalWasm(true);
		$policy->addAllowedWorkerSrcDomain("'self'");
		$policy->addAllowedWorkerSrcDomain('blob:');
		$policy->addAllowedChildSrcDomain('blob:');
		$policy->addAllowedMediaDomain('blob:');

		// Allow fetching an externally-hosted ffmpeg core/wasm, if configured.
		foreach ([$this->config->getCoreUrl(), $this->config->getWasmUrl()] as $url) {
			$origin = $this->originOf($url);
			if ($origin !== null) {
				$policy->addAllowedConnectDomain($origin);
			}
		}

		$event->addPolicy($policy);
	}

	/** Return scheme://host[:port] of an absolute http(s) URL, or null. */
	private function originOf(string $url): ?string {
		$url = trim($url);
		if ($url === '') {
			return null;
		}
		$p = parse_url($url);
		if (!isset($p['scheme'], $p['host']) || !in_array($p['scheme'], ['http', 'https'], true)) {
			return null;
		}
		$origin = $p['scheme'] . '://' . $p['host'];
		if (isset($p['port'])) {
			$origin .= ':' . $p['port'];
		}
		return $origin;
	}
}
