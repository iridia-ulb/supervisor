// https://getmdl.io/started/index.html#dynamic (call upgrade on dynamic components)


const uri = 'ws://' + location.host + '/socket';
const ws = new WebSocket(uri);

// TODO
emergency = document.getElementById('emergency-stop');
emergency.onclick = function() {}

var uiCurrentView = 'connections';
var uiTimer = null;

function setView(uiView) {
   uiCurrentView = uiView;
}

ws.onopen = function() {
   uiTimer = setInterval(function() {
      var msg = JSON.stringify({'update' : uiCurrentView})
      ws.send(msg);
   }, 250);
};

ws.onclose = function() {
   clearInterval(uiTimer);
};

ws.onmessage = function(message) {
   var update = JSON.parse(message.data);
   /* Update the title of the current interface */
   var uiTitle = document.getElementById('ui-title');
   if('title' in update) {
      uiTitle.innerHTML = update.title;
   }
   /* iterate accross existing uiCards checking for updates,
      if no update exists, remove the card from the uiContainer */
   var uiContainer = document.getElementById('ui-container');
   for(var uiCard of uiContainer.children) {
      if('cards' in update && uiCard.id in update.cards) {
         //updateCard(uiCard, update.cards[uiCard.id]);
         var uiCardUpdate = newCard(card,
            update.cards[uiCard.id].title,
            update.cards[uiCard.id].span,
            update.cards[uiCard.id].content,
            update.cards[uiCard.id].actions);
         if(uiCard.innerHTML != uiCardUpdate.innerHTML) {
            uiCard.setAttribute('class', uiCardUpdate.getAttribute('class'));
            uiCard.innerHTML = uiCardUpdate.innerHTML;
         }
      }
      else {
         /* if a card isn't in the update it should be removed from the GUI */
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

// This function returns HTMLDivElement's
function contentToHTML(content) {
   if(content.Text != null) {
      var container = document.createElement('div');
      container.setAttribute('class', 'mdl-card__supporting-text');
      container.innerHTML = content.Text;
      return container;
   }
   else if(content.Table != null) {
      var container = document.createElement('div');
      container.setAttribute('class', 'mdl-card__table');
      var table = document.createElement('table');
      table.setAttribute('class',
         'mdl-data-table mdl-js-data-table mdl-data-table--selectable');
      var tableHeader = document.createElement('thead');
      var tableHeaderRow = document.createElement('tr');
      for(var item of content.Table.header) {
         var tableHeaderRowItem = document.createElement('th');
         // sorting can be added by setting a class on 'th'
         tableHeaderRowItem.innerHTML = item;
         tableHeaderRow.appendChild(tableHeaderRowItem);
      }
      tableHeader.appendChild(tableHeaderRow);
      table.appendChild(tableHeader);
      var tableBody = document.createElement('tbody');
      for(var row of content.Table.rows) {
         var tableRow = document.createElement('tr');
         for(var element of row) {
            var tableElement = document.createElement('td');
            tableElement.innerHTML = element;
            tableRow.appendChild(tableElement);
         }
         tableBody.appendChild(tableRow)
      }
      table.appendChild(tableBody);
      container.appendChild(table);
      return container;
   }
   /* 
   else if(content.List != null) {
      var list = document.createElement('ul');
      list.setAttribute('class', 'mdl-list');
      for(var item of content.List) {
         var listItem = document.createElement('li');
         listItem.setAttribute('class', 'mdl-list__item');
         var span = document.createElement('span');
         span.setAttribute('class', 'mdl-list__item-primary-content');
         span.appendChild(contentToHTML(item));
         listItem.appendChild(span);
         list.appendChild(listItem);
      }
      return list; // element
   }
   */
   else {
      alert('Cannot convert content to HTML');
   }
}

function newCard(id, title, span, content, actions) {
   var card = document.createElement('div');
   card.setAttribute('id', id);
   var cardSpan = 'mdl-cell--' + span + '-col';
   card.setAttribute('class', 'mdl-cell dl-card mdl-shadow--2dp ' + cardSpan);
   
   var cardTitle = document.createElement('div');
   cardTitle.setAttribute('class', 'mdl-card__title');  
   var cardTitleText = document.createElement('h2');
   cardTitleText.setAttribute('class', 'mdl-card__title-text');
   cardTitleText.innerHTML = title;
   cardTitle.appendChild(cardTitleText);
   
   card.appendChild(cardTitle);

   var cardContent = contentToHTML(content);

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



