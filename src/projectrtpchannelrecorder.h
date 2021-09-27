
#ifndef PROJECTRTPCHANNELRECORDER_H
#define PROJECTRTPCHANNELRECORDER_H

#include <memory>
#include <string>

#include <boost/enable_shared_from_this.hpp>

#include "projectrtpsoundfile.h"

class channelrecorder:
  public std::enable_shared_from_this< channelrecorder >
{
public:
  typedef std::shared_ptr< channelrecorder > pointer;

  channelrecorder( std::string file );
  ~channelrecorder();

  static pointer create( std::string file );
  uint16_t poweravg( uint16_t power );
  void active( void );
  bool isactive( void ) { return this->_active & !this->completed; }

  std::string file;

  /* In packets up to MA max size (5 seconds?) */
  uint16_t poweraveragepackets;
  /* must have started for this to kick in */
  uint16_t startabovepower;
  /* must have started for this to kick in */
  uint16_t finishbelowpower;
  /* used in conjunction with finishbelowpower */
  uint32_t minduration; /* mSeconds */
  uint32_t maxduration; /* mSeconds */

  bool pause;

  int numchannels;
  soundfile::pointer sfile;

  uint16_t lastpowercalc;
  boost::posix_time::ptime created;
  boost::posix_time::ptime activeat;
  bool completed; /* indicate clean up required */

private:

  /* Rolling average of power reads */
  ma_filter powerfilter;
  bool _active;

};

#endif /* PROJECTRTPCHANNELRECORDER_H */
