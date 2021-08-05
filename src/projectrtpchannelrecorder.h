
#ifndef PROJECTRTPCHANNELRECORDER_H
#define PROJECTRTPCHANNELRECORDER_H

#include <memory>
#include <string>

#include <boost/enable_shared_from_this.hpp>

#include "projectrtpsoundfile.h"

class channelrecorder:
  public boost::enable_shared_from_this< channelrecorder >
{
public:
  channelrecorder( std::string &file );
  ~channelrecorder();
  typedef boost::shared_ptr< channelrecorder > pointer;
  static pointer create( std::string &file );
  uint16_t poweravg( uint16_t power );

  std::string file;
  std::string uuid;

/* In seconds up to MA max size (5 seconds?) */
  uint16_t poweraverageduration;
  /* must have started for this to kick in */
  uint16_t startabovepower;
  /* must have started for this to kick in */
  uint16_t finishbelowpower;
  /* used in conjunction with finishbelowpower */
  uint32_t minduration; /* mSeconds */
  uint32_t maxduration; /* mSeconds */
  int numchannels;
  soundfile::pointer sfile;
  bool active;
  uint16_t lastpowercalc;
  boost::posix_time::ptime created;
  std::string finishreason;
#warning TODO
#if 0
  controlclient::pointer control;
#endif

private:

  /* Rolling average of power reads */
  ma_filer powerfilter;

};

#endif /* PROJECTRTPCHANNELRECORDER_H */
