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
   //var msg = JSON.stringify({'emergency' : '???'})
   //ws.send('1');
}

/*
function message(data) {
   const line = document.createElement('p');
   line.innerText = data;
   chat.appendChild(line);
}
*/

var uiTimer = null;

ws.onopen = function() {
   uiTimer = setInterval(function() {
      var msg = JSON.stringify({'update' : currentWebuiView})
      ws.send(msg);
   }, 100);
};

ws.onclose = function() {
   clearInterval(uiTimer);
};

var currentWebuiView = 'connections'

function setView(webuiView) {
   currentWebuiView = webuiView;
   document.getElementById('ui-container').innerHTML = '';
}

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

ws.onmessage = function(message) {
   var update = JSON.parse(message.data);
   /* Get html elements */
   var uiTitle = document.getElementById('ui-title');
   var uiContainer = document.getElementById('ui-container');
   /* Update the title of the current interface */
   if('title' in update) {
      uiTitle.innerHTML = update.title;
   }
   /* iterate accross existing uiCards checking for updates,
      if no update exists, remove the card from the uiContainer */
   for(var uiCard of uiContainer.children) {
      if('cards' in update && uiCard.id in update.cards) {
         updateCard(uiCard, update.cards[uiCard.id]);
      }
      else {
         uiContainer.removeChild(uiCard);
      }
   }
   /* loop over the cards in the update to see if we need to
      add any new cards */
   if('cards' in update) {
      var card;
      // for X of Y
      for(card in update.cards) {
         if(update.cards.hasOwnProperty(card) &&
            document.getElementById(card) == null) {           
            var newUiCard = newCard(card,
                                    update.cards[card].title,
                                    update.cards[card].span,
                                    update.cards[card].content,
                                    update.cards[card].actions)
            uiContainer.appendChild(newUiCard)
         }
      }
   }
};

function updateCard(uiCard, update) {
   var title =
      uiCard.getElementsByClassName('mdl-card__title-text')[0];
   if(title.innerHTML != update.title) {
      title.innerHTML = update.title;
   }
   var content =
      uiCard.getElementsByClassName('mdl-card__supporting-text')[0];
   if(content.innerHTML != update.content) {
      content.innerHTML = update.content;
   }
   // TODO actions?
}

function newCard(id, title, span, content, actions) {
   var card = document.createElement('div');
   card.setAttribute('id', id);
   card.setAttribute('class', 'mdl-cell mdl-cell--4-col mdl-card mdl-shadow--2dp');
   
   cardTitle = document.createElement('div');
   cardTitle.setAttribute('class', 'mdl-card__title');  
   cardTitleText = document.createElement('h2');
   cardTitleText.setAttribute('class', 'mdl-card__title-text');
   cardTitleText.innerHTML = title;
   cardTitle.appendChild(cardTitleText);
   
   card.appendChild(cardTitle);
   
   cardContent = document.createElement('div');
   cardContent.setAttribute('class', 'mdl-card__supporting-text');
   cardContent.innerHTML = content;

   card.appendChild(cardContent);
   
   cardActions = document.createElement('div');
   cardActions.setAttribute('class', 'mdl-card__actions mdl-card--border');
   
   for(var action of actions) {
      cardAction = document.createElement('a');
      cardAction.setAttribute('class', 'mdl-button mdl-button--colored mdl-js-button mdl-js-ripple-effect');
      cardAction.innerHTML = action;
      cardAction.onclick = function() {
         var msg = JSON.stringify({'drone' : [id, "Power on UpCore"]})
         ws.send(msg);
      };
      cardActions.appendChild(cardAction);
   }

   card.appendChild(cardActions);
   
   return card;
}



