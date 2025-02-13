

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

#include <functional>

#include "projectrtppacket.h"
#include "projectrtpcodecx.h"

/* number of soundfile async buffers */
#define SOUNDFILENUMBUFFERS 16
#define MAXNUMBEROFCHANNELS 2

typedef struct {
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

class soundfile {
public:
  soundfile( int fromfile = -1 );
  ~soundfile();

  soundfile( const soundfile& ) = delete;              // copy ctor
  soundfile( soundfile&& ) = delete;                   // move ctor
  soundfile& operator=( const soundfile& ) = delete;   // copy assignment
  soundfile& operator=( soundfile&& ) = delete;        // move assignment

  inline uint16_t getwavformat( void ) { return this->ourwavheader.audio_format; }
  /* Gets the current wav file format and converts to equiv RTP payload type */
  uint8_t getwavformattopt( void );

  static int getsampleratefrompt( uint8_t pt );
  uint32_t getwriteduration( void ); /* Return umber of mS since start of recording (recorded data) */

  std::string &geturl( void ) { return this->url; };

protected:
  int file;
  std::string url;
  wavheader ourwavheader;

  /* asynchronous variables */
  u_int8_t currentcbindex;
  aiocb cbwavheader;
  aiocb cbwavblock[ SOUNDFILENUMBUFFERS ];

  /* buffer for data */
  uint8_t *buffer;

  std::atomic_bool filelock;
};


class soundfilereader : public soundfile {
public:
  soundfilereader( std::string &url );
  ~soundfilereader();

  soundfilereader( const soundfilereader& ) = delete;              // copy ctor
  soundfilereader( soundfilereader&& ) = delete;                   // move ctor
  soundfilereader& operator=( const soundfilereader& ) = delete;   // copy assignment
  soundfilereader& operator=( soundfilereader&& ) = delete;        // move assignment

  typedef std::shared_ptr< soundfilereader > pointer;
  static pointer create( std::string url );

  bool read( rawsound &out );
  bool complete( void );

  void setposition( long mseconds );
  long getposition( void );

  inline bool isopen( void ) { return this->file != -1; }

private:
  long offtomsecs( void );

  int blocksize;
  bool badheader;
  bool headerread;
  bool bodyread;
  long initseekmseconds;
  int ploadtype;
};


class soundfilewriter : public soundfile {
public:
  soundfilewriter( std::string &url, int16_t numchannels, int32_t samplerate );
  ~soundfilewriter();

  typedef std::shared_ptr< soundfilewriter > pointer;
  static pointer create( std::string &url, int16_t numchannels, int32_t samplerate );

  soundfilewriter( const soundfilewriter& ) = delete;              // copy ctor
  soundfilewriter( soundfilewriter&& ) = delete;                   // move ctor
  soundfilewriter& operator=( const soundfilewriter& ) = delete;   // copy assignment
  soundfilewriter& operator=( soundfilewriter&& ) = delete;        // move assignment

  bool write( codecx &in, codecx &out );

private:
  int32_t tickcount;

};


void initwav( wavheader *, int samplerate = 8000 );

#ifdef NODE_MODULE
#include <node_api.h>
void initrtpsoundfile( napi_env env, napi_value &result );
#endif



#endif /* PROJECTRTPSOUNDFILE_H */
