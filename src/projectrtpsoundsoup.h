


#ifndef PROJECTRTPSOUNDSOUP_H
#define PROJECTRTPSOUNDSOUP_H

#include <string>
#include <memory>

#include <vector>

#include "projectrtpsoundfile.h"
#include "json.hpp"

class soundsoupfile
{
public:
  soundsoupfile();
  ~soundsoupfile();
  int start;
  int stop;
  int loopcount;
  int maxloop;
  soundfile::pointer sf;
};


typedef std::vector< soundsoupfile > soundsoupfiles;

/*!md
# soundsoup
A soup of sounds! In other words a form of macro player. We receive a JSON object instructing us what to play.
```json
{
  files:[
    { wav: file://unknnownformat.wav, ilbc: file://fileilbc.wav, l168k: file://filel188k.wav, start: 0, stop: 100, loop: 5 },
    { l168k: file://secondfile.wav }
  ],
  loop: true
}
```

Description of params
|Param|Description|
|-|-|
|loop|Either a bool (continuous loop) or an int for number of loops|
|start, stop|Not the hole file - just start at seconds and finish at seconds|
|wav, ilbc, l168k, l1616k, pcma, pcmu, g722|Filenames containing the encoding - this helps choose which to play. All files should contain the same recording just in a different format|

Warning: no checking is done to ensure a file exists. So it is up to the caller to ensure that the filenames are valid and the format correct. If the format of the file is incorrect but the sound is there then it will use a codec to play - int turn more CPU - defeating the oject of having multiple files. If you are unsure which to use use the wav entry.

When we receive an update - we don't want to abruptly end the playback - if possible finish off the current file then continue. But it will difficult to figure out where in the new instruction where it is supposed to be playing.

So, if the current playback is in index file 1, this file has the same filename as the one in the incoming instruction then this is where we continue from. Otherwise we start at 0 again.
*/

class soundsoup
//  : std::enable_shared_from_this< soundsoup >
{
public:
  soundsoup( void );
  ~soundsoup();

  typedef std::shared_ptr< soundsoup > pointer;
  static pointer create( void );

  void config( JSON::Object &json, int format );
  bool read( rawsound &out );

private:
  std::string *getpreferredfilename( JSON::Object &file, int format );
  void plusone( soundsoupfile &playing );

  /* This is used to choose the best format file */
  int loopcount;
  size_t currentfile;
  soundsoupfiles files;
};

#endif  /* PROJECTRTPSOUNDSOUP_H */
