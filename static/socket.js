// https://getmdl.io/started/index.html#dynamic (call upgrade on dynamic components)

const uri = 'ws://' + location.host + '/socket';

var uiCurrentView = 'Connections';
var uiTimer = null;

function setView(uiView) {
   uiCurrentView = uiView;
}

let ws = new WebSocket(uri);

ws.onopen = function() {
   document.getElementById('offline').style.display = 'None'
   uiTimer = setInterval(function() {
      var message = JSON.stringify({
         type: 'update',
         tab : uiCurrentView
      });
      ws.send(message);
   }, 250);
};

ws.onclose = function() {
   document.getElementById('offline').style.display = ''
   clearInterval(uiTimer);
};

ws.onmessage = function(message) {
   let update = JSON.parse(message.data);
   /* Update the title of the current interface */
   let uiTitle = document.getElementById('ui-title');
   if('title' in update) {
      uiTitle.innerHTML = update.title;
   }
   if('cards' in update) {
      let uiContainer = document.getElementById('ui-container');
      /* iterate over the existing uiCards (i.e., HTMLDivElement's) */
      for(let uiCard of uiContainer.children) {
         /* search for a card by its UUID */
         let cardFound = false;
         for(card of update.cards) {
            if('uuid' in card && card.uuid == uiCard.id) {
               /* card was found, generate a new card and compare */
               let uiCardUpdate = 
                  newCard(card.uuid, card.title, card.span, card.content, card.actions);
               if(uiCard.outerHTML != uiCardUpdate.outerHTML) {
                  uiContainer.replaceChild(uiCardUpdate, uiCard);
               }
               cardFound = true;
               break;
            }
         }
         /* if the card was not in the update, we remove it from the ui */
         if(!cardFound) {
            uiContainer.removeChild(uiCard);
         }
      }
      for(card of update.cards) {
         if('uuid' in card && document.getElementById(card.uuid) == null) {
            var newUiCard =
               newCard(card.uuid, card.title, card.span, card.content, card.actions);
            uiContainer.appendChild(newUiCard);
         }
      }
   }
};

// TODO
emergency = document.getElementById('emergency-stop');
emergency.onclick = function() {}

// This function returns HTMLDivElement's
function contentToHTML(content) {
   if(content.text != null) {
      var container = document.createElement('div');
      container.setAttribute('class', 'mdl-card__supporting-text');
      container.innerHTML = content.text;
      return container;
   }
   else if(content.table != null) {
      var container = document.createElement('div');
      container.setAttribute('class', 'mdl-card__table');
      var table = document.createElement('table');
      table.setAttribute('class',
         'mdl-data-table mdl-js-data-table mdl-data-table--selectable');
      var tableHeader = document.createElement('thead');
      var tableHeaderRow = document.createElement('tr');
      for(var item of content.table.header) {
         var tableHeaderRowItem = document.createElement('th');
         // sorting can be added by setting a class on 'th'
         tableHeaderRowItem.innerHTML = item;
         tableHeaderRow.appendChild(tableHeaderRowItem);
      }
      tableHeader.appendChild(tableHeaderRow);
      table.appendChild(tableHeader);
      var tableBody = document.createElement('tbody');
      for(var row of content.table.rows) {
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
   else if(content.list != null) {
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

/* factory for sending commands to the backend */
function sendActionFactory(type, uuid, action) {
   return function() {
      var message = JSON.stringify({
         type: type,
         action: action,
         uuid: uuid,
      });
      ws.send(message);
   }
}

function newCard(uuid, title, span, content, controls) {
   /* create card */
   var card = document.createElement('div');
   card.setAttribute('id', uuid);
   var cardSpan = 'mdl-cell--' + span + '-col';
   card.setAttribute('class', 'mdl-cell dl-card mdl-shadow--2dp ' + cardSpan);
   /* create title */
   var cardTitle = document.createElement('div');
   cardTitle.setAttribute('class', 'mdl-card__title');
   var cardTitleText = document.createElement('h2');
   cardTitleText.setAttribute('class', 'mdl-card__title-text');
   cardTitleText.innerHTML = title;
   cardTitle.appendChild(cardTitleText);
   card.appendChild(cardTitle);
   /* create content */
   for(section of content) {
      var cardSection = contentToHTML(section);
      card.appendChild(cardSection);
   }
   /* create controls */
   cardControls = document.createElement('div');
   cardControls.setAttribute('class', 'mdl-card__actions mdl-card--border');
   for(var control of controls) {
      if(control.type == 'software' && control.action == 'Upload') {
         cardControlInput = document.createElement('input');
         cardControlInput.setAttribute('type', 'file');
         cardControlInput.setAttribute('multiple', true);
         cardControlInput.setAttribute('id', uuid + '_upload');
         cardControlInput.setAttribute('style', 'display: none');
         cardControlInput.onchange = function() {
            uploadInput = document.getElementById(uuid + '_upload');
            for (var i = 0; i < uploadInput.files.length; i++) {
               const file = uploadInput.files[i];
               const reader = new FileReader();
               reader.onload = function(ev) {
                  var message = JSON.stringify({
                     type: 'software',
                     action: 'Upload',
                     file: [file.name, ev.target.result],
                     uuid: uuid,
                  });
                  ws.send(message);
               };
               reader.readAsDataURL(file);
            }
         };
         cardControl = document.createElement('label');
         cardControl.setAttribute('for', uuid + '_upload');
         cardControl.setAttribute('class', 'mdl-button mdl-button--colored mdl-js-button mdl-js-ripple-effect');
         cardControl.innerHTML = control.action;
         cardControl.appendChild(cardControlInput);
      }
      else {
         cardControl = document.createElement('a');
         cardControl.setAttribute('class', 'mdl-button mdl-button--colored mdl-js-button mdl-js-ripple-effect');
         cardControl.innerHTML = control.action;
         cardControl.onclick = sendActionFactory(control.type, uuid, control.action);
      }
      cardControls.appendChild(cardControl);
   }
   card.appendChild(cardControls);
   return card;
}

/* connect to the backend */
connect()

