// Motiv: meeting awareness popup logic (see meeting-prompt.html).
(function () {
  var q = new URLSearchParams(location.search);
  var kind = q.get('kind') === 'stop' ? 'stop' : 'start';
  var app = q.get('app') || 'a meeting';
  var name = q.get('title') || '';
  var $ = function (id) { return document.getElementById(id); };

  if (kind === 'start') {
    $('title').textContent = 'Record this ' + app + ' call?';
    $('sub').textContent = name || 'Motiv noticed a meeting started.';
    $('go').textContent = 'Record';
    $('dismiss').textContent = 'Not now';
    $('never').textContent = "Don't ask";
  } else {
    $('title').textContent = 'The ' + app + ' call ended';
    $('sub').textContent = 'Stop recording and save it?';
    $('go').textContent = 'Stop & save';
    $('dismiss').textContent = 'Keep recording';
    $('never').textContent = "Don't ask";
  }

  function respond(action) {
    var t = window.__TAURI_INTERNALS__;
    if (t && t.invoke) {
      t.invoke('meeting_prompt_respond', { action: action }).catch(function (e) { console.error(e); });
    } else {
      window.close();
    }
  }
  $('go').onclick = function () { respond(kind === 'start' ? 'record' : 'stop'); };
  $('dismiss').onclick = function () { respond('dismiss'); };
  $('never').onclick = function () { respond('never'); };
  document.addEventListener('keydown', function (e) {
    if (e.key === 'Escape') respond('dismiss');
    if (e.key === 'Enter') $('go').click();
  });
})();
