

#ifndef PROJECTRTPRINGBUFFER_H
#define PROJECTRTPRINGBUFFER_H

#include <cstdint>
#include <cstring>
#include <vector>

/*
# pcmringbuffer

Circular buffer for L16 PCM samples. Used to pre-buffer incoming audio
during playback so that barge-in recording can include audio from before
the interrupt was detected.

Only accessed from the handletick thread - no locking needed.

At 16kHz wideband: 320 samples per 20ms tick.
25 ticks = 500ms = 8000 samples.
At 8kHz narrowband: 160 samples per 20ms tick.
25 ticks = 500ms = 4000 samples.

We allocate for the wideband case.
*/

#define PREBUFMAXPACKETS 25
#define PREBUFMAXSAMPLES ( 320 * PREBUFMAXPACKETS )

class pcmringbuffer {
public:
  pcmringbuffer() : head( 0 ), count( 0 ) {}

  void push( int16_t *data, size_t samples ) {
    if( nullptr == data || 0 == samples ) return;

    for( size_t i = 0; i < samples; i++ ) {
      buf[ head ] = data[ i ];
      head = ( head + 1 ) % PREBUFMAXSAMPLES;
      if( count < PREBUFMAXSAMPLES ) {
        count++;
      }
    }
  }

  /* drain all buffered samples in chronological order into out, return sample count */
  size_t drain( std::vector< int16_t > &out ) {
    if( 0 == count ) return 0;

    out.resize( count );
    size_t start = ( head + PREBUFMAXSAMPLES - count ) % PREBUFMAXSAMPLES;
    for( size_t i = 0; i < count; i++ ) {
      out[ i ] = buf[ ( start + i ) % PREBUFMAXSAMPLES ];
    }

    size_t drained = count;
    count = 0;
    head = 0;
    return drained;
  }

  void clear( void ) {
    head = 0;
    count = 0;
  }

  size_t size( void ) { return count; }

private:
  int16_t buf[ PREBUFMAXSAMPLES ];
  size_t head;
  size_t count;
};

#endif /* PROJECTRTPRINGBUFFER_H */
