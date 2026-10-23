<script lang="ts" module>
  // 16px line icons, drawn with currentColor. Filled ones set fill themselves.
  const F = 'fill="currentColor" stroke="none"';
  const ICONS: Record<string, string> = {
    server: '<rect x="2" y="2.5" width="12" height="4" rx="1"/><rect x="2" y="9.5" width="12" height="4" rx="1"/><path d="M4.5 4.5h.01M4.5 11.5h.01"/>',
    database: '<ellipse cx="8" cy="4" rx="5" ry="2"/><path d="M3 4v8c0 1.1 2.2 2 5 2s5-.9 5-2V4M3 8c0 1.1 2.2 2 5 2s5-.9 5-2"/>',
    folder: '<path d="M1.5 4a1 1 0 0 1 1-1h3.2l1.5 1.5h6.3a1 1 0 0 1 1 1V12a1 1 0 0 1-1 1h-11a1 1 0 0 1-1-1z"/>',
    table: '<rect x="2" y="3" width="12" height="10" rx="1"/><path d="M2 6.5h12M2 10h12M6.5 6.5V13"/>',
    view: '<path d="M1.5 8S4 3.5 8 3.5 14.5 8 14.5 8 12 12.5 8 12.5 1.5 8 1.5 8z"/><circle cx="8" cy="8" r="2"/>',
    procedure: '<path d="M6 2.5c-1.5 0-1.5 1-1.5 2.5v1.5C4.5 7.5 4 8 3 8c1 0 1.5.5 1.5 1.5V11c0 1.5 0 2.5 1.5 2.5M10 2.5c1.5 0 1.5 1 1.5 2.5v1.5c0 1 .5 1.5 1.5 1.5-1 0-1.5.5-1.5 1.5V11c0 1.500 0 2.5-1.5 2.5"/>',
    function: '<path d="M9.5 2.5c-2 0-1.500 1.500-2 3.500L6.500 11c-.5 2-1 2.500-2.500 2.500M4.500 7h5M9 10l3 3.500M12 10l-3 3.500"/>',
    column: '<rect x="5" y="2" width="6" height="12" rx="1"/><path d="M5 6h6M5 10h6"/>',
    key: '<circle cx="5" cy="8" r="2.500"/><path d="M7.500 8H14M11.500 8v2.500M14 8v2"/>',
    lock: '<rect x="3" y="7" width="10" height="7" rx="1.500"/><path d="M5.500 7V5a2.500 2.500 0 0 1 5 0v2"/>',
    unlock: '<rect x="3" y="7" width="10" height="7" rx="1.500"/><path d="M5.500 7V5a2.500 2.500 0 0 1 4.800-1"/>',
    run: `<path ${F} d="M4.500 2.800v10.400L13 8z"/>`,
    stop: `<rect ${F} x="3.500" y="3.500" width="9" height="9" rx="1.500"/>`,
    plus: '<path d="M8 3v10M3 8h10"/>',
    refresh: '<path d="M13 8a5 5 0 1 1-1.600-3.700M13 2.500v3h-3"/>',
    todo: '<rect x="2.500" y="2.500" width="11" height="11" rx="2"/><path d="M5.500 8.200l1.700 1.700 3.300-3.600"/>',
    chevron: '<path d="M6 4l4 4-4 4"/>',
    more: `<path ${F} d="M3 7a1 1 0 1 0 0 2 1 1 0 0 0 0-2zM8 7a1 1 0 1 0 0 2 1 1 0 0 0 0-2zM13 7a1 1 0 1 0 0 2 1 1 0 0 0 0-2z"/>`,
    close: '<path d="M4 4l8 8M12 4l-8 8"/>',
    edit: '<path d="M2.500 13.500l.5-3L11 2.500l2.500 2.500L5.500 13z"/>',
    trash: '<path d="M3 4.500h10M6.500 4.500V3h3v1.500M4.500 4.500l.5 8.500h6l.5-8.500"/>',
    copy: '<rect x="5.500" y="5.500" width="8" height="8" rx="1.500"/><path d="M10.500 5.500v-2a1 1 0 0 0-1-1h-6a1 1 0 0 0-1 1v6a1 1 0 0 0 1 1h2"/>',
    file: '<path d="M4 2h5l3.500 3.500V13a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V3a1 1 0 0 1 1-1zM9 2v3.500h3.500M5.500 8.500h5M5.500 11h5"/>',
    check: '<path d="M3 8.500l3.500 3.500L13 4.500"/>',
  };
</script>

<script lang="ts">
  let { name, size = 16 }: { name: string; size?: number } = $props();
</script>

<svg width={size} height={size} viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">{@html ICONS[name]}</svg>
