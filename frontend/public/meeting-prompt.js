// Motiv: meeting awareness popup logic (see meeting-prompt.html).
(function () {
  var q = new URLSearchParams(location.search);
  var kind = q.get('kind') === 'stop' ? 'stop' : 'start';
  var app = q.get('app') || 'Online';
  var name = q.get('title') || '';
  var $ = function (id) { return document.getElementById(id); };

  if (kind === 'start') {
    $('title').textContent = app + ' meeting detected, start recording?';
    $('sub').textContent = name || 'Motiv will transcribe and summarize it on this computer.';
    $('go').textContent = 'Start recording';
    $('dismiss').textContent = 'Not now';
    $('never').textContent = "Don't ask again";
  } else {
    $('card').classList.add('ended');
    $('title').textContent = app + ' meeting ended, stop recording?';
    $('sub').textContent = name || 'Motiv will save the recording and transcript.';
    $('go').textContent = 'Stop & save';
    $('dismiss').textContent = 'Keep recording';
    $('never').textContent = "Don't ask again";
  }
  document.title = $('title').textContent;

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
  $('close').onclick = function () { respond('dismiss'); };
  $('never').onclick = function () { respond('never'); };
  document.addEventListener('keydown', function (e) {
    if (e.key === 'Escape') respond('dismiss');
    if (e.key === 'Enter') $('go').click();
  });
})();
