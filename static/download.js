function save_frame(uri) {
  var link = document.createElement('a');
  link.href = uri;
  link.download = 'frame.jpg';
  document.body.appendChild(link);
  link.click();
  document.body.removeChild(link);
}

