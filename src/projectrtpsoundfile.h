

#ifndef PROJECTRTPSOUNDFILE_H
#define PROJECTRTPSOUNDFILE_H

#include <string>

#include <memory>


#include <sys/types.h>
#include <sys/stat.h>
#include <aio.h>
#include <errno.h>
#include <fcntl.h>

#include <stdint.h>

#include "projectrtppacket.h"
#include "projectrtpcodecx.h"

/* min 2 for write buffers on 2 channel audio */
#define SOUNDFILENUMBUFFERS 2

/*!md

*/

typedef struct
{
    /* RIFF Header */
    uint8_t riff_header[ 4 ]; /* Contains "RIFF" */
    uint32_t chunksize; /* Size of the wav portion of the file, which follows the first 8 bytes. File size - 8 */

    uint8_t wave_header[ 4 ]; /* Contains "WAVE" */

    /* Format Header */
    uint8_t fmt_header[ 4 ]; /* Contains "fmt " (includes trailing space) */
    int32_t fmt_chunk_size; /* Should be 16 for PCM */
    uint16_t audio_format; /* Should be 1 for PCM. 3 for IEEE Float */
    int16_t num_channels;
    int32_t sample_rate;
    int32_t byte_rate; /* Number of bytes per second. sample_rate * num_channels * Bytes Per Sample */
    int16_t sample_alignment; /* num_channels * Bytes Per Sample */
    int16_t bit_depth; /* Number of bits per sample */

    /* Data */
    uint8_t data_header[ 4 ]; /* Contains "data" */
    /* int32_t data_bytes;  Number of bytes in data. Number of samples * num_channels * sample byte size */
    /* Remainder of wave file is bytes */

    uint32_t subchunksize;

} wavheader;

/* Actual value in the wav header */
#define  WAVE_FORMAT_PCM 0x0001
#define  WAVE_FORMAT_ALAW 0x0006
#define  WAVE_FORMAT_MULAW 0x0007
#define  WAVE_FORMAT_POLYCOM_G722 0xA112 /* Polycom - there are other versions */
#define  WAVE_FORMAT_GLOBAL_IP_ILBC 0xA116 /* Global IP */

class soundfile
{
public:
  soundfile( std::string &url );
  soundfile( std::string &url, uint16_t audio_format, int16_t numchannels, int32_t samplerate );
  ~soundfile();

  typedef std::shared_ptr< soundfile > pointer;
  static pointer create( std::string &url );
  static pointer create( std::string &url, uint16_t audio_format, int16_t numchannels, int32_t samplerate );
  std::string &geturl( void ) { return this->url; };

  bool read( rawsound &out );
  bool write( codecx &in, codecx &out );

  void setposition( long seconds );
  long getposition( void );
  uint32_t getwriteduration( void ); /* Return umber of mS since start of recording (recorded data) */
  bool complete( void );
  inline bool isopen( void ) { return this->file != -1; }

  inline uint16_t getwavformat( void ) { return this->ourwavheader.audio_format; }
  /* Gets the current wav file format and converts to equiv RTP payload type */
  uint8_t getwavformattopt( void );

  /* converts from values like PCMUPAYLOADTYPE to WAVE_FORMAT_MULAW */
  static uint16_t wavformatfrompt( uint8_t pt );
  static int getsampleratefrompt( uint8_t pt );

private:
  long offtomsecs( void );
  int file;
  std::string url;
  uint8_t *readbuffer;
  int readbuffercount;
  int currentreadindex;
  aiocb cbwavheader;
  wavheader ourwavheader;
  aiocb cbwavblock[ SOUNDFILENUMBUFFERS ];

  bool opened;
  bool badheader;

  long newposition;
  bool headerread;

  /* For writing */
  uint8_t *writebuffer;
  int currentwriteindex;
  int32_t tickcount;
};




void wavinfo( const char *file );
void initwav( wavheader *, int samplerate = 8000 );


#endif /* PROJECTRTPSOUNDFILE_H */
