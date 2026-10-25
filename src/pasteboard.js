ObjC.import('AppKit');
function run(argv) {
  const board = argv.length ? $.NSPasteboard.pasteboardWithName(argv[0]) : $.NSPasteboard.generalPasteboard;
  const urls = board.readObjectsForClassesOptions($([$.NSURL]), $({ NSPasteboardURLReadingFileURLsOnlyKey: true }));
  if (urls.count > 0) {
    return ['files'].concat(urls.js.map(url => url.absoluteString.js)).join('\n');
  }
  if ($.NSImage.canInitWithPasteboard(board)) {
    let png = board.dataForType('public.png');
    if (png.isNil()) {
      const rep = $.NSBitmapImageRep.imageRepWithData($.NSImage.alloc.initWithPasteboard(board).TIFFRepresentation);
      png = rep.representationUsingTypeProperties($.NSBitmapImageFileTypePNG, $({}));
    }
    return 'image\n' + png.base64EncodedStringWithOptions(0).js;
  }
  const text = board.stringForType($.NSPasteboardTypeString);
  return text.isNil() ? '' : 'text\n' + text.js;
}
