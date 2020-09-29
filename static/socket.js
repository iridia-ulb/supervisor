// https://getmdl.io/started/index.html#dynamic (call upgrade on dynamic components)


const uri = 'ws://' + location.host + '/socket';
const ws = new WebSocket(uri);

theButton = document.getElementById('emergency-stop');
//const text = document.getElementById('text');

/*
thebutton.addEventListener('ValueChange', function() {
   alert('changed!');
});
*/

theButton.onclick = function() {
   ws.send('1');
}

/*
function message(data) {
   const line = document.createElement('p');
   line.innerText = data;
   chat.appendChild(line);
}
*/

var gui_timer = null;

ws.onopen = function() {
   gui_timer = setInterval(function(){ ws.send('connections'); }, 100);
   console.log(gui_timer)
};

/*
<div class="mdl-cell mdl-cell--4-col mdl-card mdl-shadow--2dp">
  <div class="mdl-card__title">
    <h2 class="mdl-card__title-text">Drone #0001</h2>
  </div>
  <div class="mdl-card__supporting-text">Text</div>
  <div class="mdl-card__actions mdl-card--border">
    <a class="mdl-button mdl-button--colored mdl-js-button mdl-js-ripple-effect">Power On</a>
  </div>
</div>
*/

ws.onmessage = function(msg) {
   var content = JSON.parse(msg.data);
   console.log(content)
   
   var title = document.getElementById('ui-title');
   title.innerHTML = content.title;
   
   var container = document.getElementById('ui-container');

   // remove previous children
   while (container.firstChild) {
      container.removeChild(container.lastChild);
   }
   
   var card = document.createElement('div');
   card.setAttribute('class', 'mdl-cell mdl-cell--4-col mdl-card mdl-shadow--2dp');

   cardTitle = document.createElement('div');
   cardTitle.setAttribute('class', 'mdl-card__title');  
   cardTitleText = document.createElement('h2');
   cardTitleText.setAttribute('class', 'mdl-card__title-text');
   cardTitleText.innerHTML = content.cards[0].title;
   cardTitle.appendChild(cardTitleText);
   
   card.appendChild(cardTitle);
   
   cardText = document.createElement('div');
   cardText.setAttribute('class', 'mdl-card__supporting-text');
   cardText.innerHTML = content.cards[0].text;

   card.appendChild(cardText);
   
   cardActions = document.createElement('div');
   cardActions.setAttribute('class', 'mdl-card__actions mdl-card--border');
   
   // for each
   cardAction = document.createElement('a');
   cardAction.setAttribute('class', 'mdl-button mdl-button--colored mdl-js-button mdl-js-ripple-effect');
   cardAction.innerHTML = content.cards[0].actions[0];
   
   cardActions.appendChild(cardAction);
   card.appendChild(cardActions);
   
   container.appendChild(card);

};

/*
function addCard(card) {

}
*/

ws.onclose = function() {
   console.log(gui_timer)
   clearInterval(gui_timer);
};

